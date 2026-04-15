//! # 倒排索引（Inverted Index）— 基于 Tag 的快速查询
//!
//! ## 设计动机
//!
//! TSDB 中每个时间序列由 **measurement + tags** 唯一标识。
//! 查询时经常需要按 tag 条件过滤（如 `WHERE host='server01' AND region='us-west'`），
//! 倒排索引将这种 "tag value → series_id_list" 的映射关系预先建立好，
//! 避免全表扫描。
//!
//! ## 数据结构
//!
//! ```text
//! InvertedIndex
//! ├── postings: HashMap<String, RoaringBitmap>
//! │   ┌─────────────────────────────────────┐
//! │   │ "host=server01"  → {1, 3, 5, 7}    │  RoaringBitmap 压缩存储
//! │   │ "region=us-west" → {1, 2, 4}       │  支持高效的位运算
//! │   │ "host=server02"  → {2, 4, 6}       │
//! │   └─────────────────────────────────────┘
//! │
//! └── series_tags: HashMap<SeriesId, Vec<(String, String)>>
//!     ┌──────────────────────────────────┐
//!     │ 1 → [("host","s01"), ("region","us-west")] │  用于删除时清理
//!     │ 2 → [("host","s02"), ("region","us-west")] │
//!     └──────────────────────────────────┘
//! ```
//!
//! ## 查询操作
//!
//! | 操作 | 方法 | 说明 |
//! |------|------|------|
//! | 精确匹配 | `query_exact()` | 单个 tag key=value → SeriesId 集合 |
//! | 交集查询 | `query_intersection()` | 多个条件 AND → 同时满足的 SeriesId |
//! | 并集查询 | `query_union()` | 多个条件 OR → 任一满足的 SeriesId |
//!

use roaring::RoaringBitmap;
use std::collections::HashMap;
use tsdb_types::model::SeriesId;

/// 倒排索引 — 基于 Tag 的序列查找索引
///
/// 使用 **RoaringBitmap**（压缩位图）存储每个 tag 键值对对应的 SeriesId 集合。
/// RoaringBitmap 相比普通 HashSet/Vec 具有以下优势：
/// - **内存高效**：压缩率可达 10:1 以上（尤其适合稀疏 ID 分布）
/// - **位运算快**：交集/并集/差集均为 O(n) 级别
/// - **可序列化**：支持二进制持久化到磁盘
///
/// ## 线程安全
///
/// 当前实现非线程安全，需在调用方加锁保护（或后续升级为 RwLock）。
pub struct InvertedIndex {
    /// 倒排列表：`tag_key=tag_value` → 匹配的 SeriesId 位图
    postings: HashMap<String, RoaringBitmap>,
    /// 反向映射：SeriesId → 该序列的所有 Tag 对（用于删除时精确清理）
    series_tags: HashMap<SeriesId, Vec<(String, String)>>,
}

impl InvertedIndex {
    /// 创建新的空倒排索引实例
    pub fn new() -> Self {
        Self {
            postings: HashMap::new(),
            series_tags: HashMap::new(),
        }
    }

    /// 向索引中添加一个新时间序列及其标签
    ///
    /// 为该序列的每个 (key, value) 标签对创建倒排条目：
    /// - 在 `postings` 中将 series_id 加入对应 tag 的 RoaringBitmap
    /// - 在 `series_tags` 中记录该序列的所有标签（用于后续删除）
    ///
    /// # 参数
    /// - `series_id`: 时间序列的唯一标识符
    /// - `tags`: 该序列的标签键值对列表
    pub fn add_series(&mut self, series_id: SeriesId, tags: &[(String, String)]) {
        self.series_tags.insert(series_id, tags.to_vec());
        for (key, value) in tags {
            let posting_key = format!("{}={}", key, value);
            self.postings
                .entry(posting_key)
                .or_insert_with(RoaringBitmap::new)
                .insert(series_id as u32);
        }
    }

    /// 从索引中移除一个时间序列及其所有标签关联
    ///
    /// 清理步骤：
    /// 1. 从 `series_tags` 获取该序列的所有标签
    /// 2. 从每个标签对应的 RoaringBitmap 中移除该 series_id
    /// 3. 如果某标签的 Bitmap 变为空，则从 postings 中彻底删除该条目
    ///
    /// # 参数
    /// - `series_id`: 待移除的时间序列 ID
    pub fn remove_series(&mut self, series_id: SeriesId) {
        if let Some(tags) = self.series_tags.remove(&series_id) {
            for (key, value) in tags {
                let posting_key = format!("{}={}", key, value);
                if let Some(bitmap) = self.postings.get_mut(&posting_key) {
                    bitmap.remove(series_id as u32);
                    if bitmap.is_empty() {
                        self.postings.remove(&posting_key);
                    }
                }
            }
        }
    }

    /// 精确查询单个 tag 键值对匹配的所有序列
    ///
    /// # 参数
    /// - `tag_key`: 标签键（如 `"host"`）
    /// - `tag_value`: 标签值（如 `"server01"`）
    ///
    /// # 返回
    /// 匹配的 SeriesId 集合（RoaringBitmap，可能为空）
    pub fn query_exact(&self, tag_key: &str, tag_value: &str) -> RoaringBitmap {
        let posting_key = format!("{}={}", tag_key, tag_value);
        self.postings.get(&posting_key).cloned().unwrap_or_default()
    }

    /// 交集查询 — 多个 tag 条件同时满足（AND 语义）
    ///
    /// 将各条件对应的 RoaringBitmap 逐个做位与（&=）运算，
    /// 最终结果为同时满足所有条件的 SeriesId 集合。
    ///
    /// # 参数
    /// - `filters`: 标签过滤条件列表，所有条件必须同时满足
    ///
    /// # 返回
    /// 交集结果（RoaringBitmap）
    pub fn query_intersection(&self, filters: &[(String, String)]) -> RoaringBitmap {
        if filters.is_empty() {
            return RoaringBitmap::new();
        }

        let bitmaps: Vec<RoaringBitmap> = filters
            .iter()
            .map(|(k, v)| self.query_exact(k, v))
            .collect();

        let mut result = bitmaps[0].clone();
        for bitmap in &bitmaps[1..] {
            result &= bitmap;
        }
        result
    }

    /// 并集查询 — 任一 tag 条件满足即可（OR 语义）
    ///
    /// 将各条件对应的 RoaringBitmap 逐个做位或（|=）运算，
    /// 最终结果为满足任一条件的 SeriesId 集合。
    pub fn query_union(&self, filters: &[(String, String)]) -> RoaringBitmap {
        let mut result = RoaringBitmap::new();
        for (k, v) in filters {
            let posting_key = format!("{}={}", k, v);
            if let Some(bitmap) = self.postings.get(&posting_key) {
                result |= bitmap;
            }
        }
        result
    }

    /// 返回当前已索引的时间序列数量
    pub fn series_count(&self) -> usize {
        self.series_tags.len()
    }

    /// 返回当前倒排列表中的不同 tag 键值对数量
    pub fn posting_count(&self) -> usize {
        self.postings.len()
    }

    /// 获取指定序列的所有标签
    ///
    /// # 参数
    /// - `series_id`: 时间序列 ID
    ///
    /// # 返回
    /// 该序列的标签列表引用，若不存在则返回 None
    pub fn get_series_tags(&self, series_id: SeriesId) -> Option<&[(String, String)]> {
        self.series_tags.get(&series_id).map(|v| v.as_slice())
    }

    /// 获取所有已注册的 SeriesId 集合
    ///
    /// 用于全量扫描等场景。
    pub fn all_series_ids(&self) -> RoaringBitmap {
        let mut bitmap = RoaringBitmap::new();
        for &id in self.series_tags.keys() {
            bitmap.insert(id as u32);
        }
        bitmap
    }

    /// 将倒排索引序列化为二进制数据（用于持久化到 RocksDB）
    ///
    /// ## 序列化格式
    ///
    /// ```text
    /// [postings_count: u32]
    ///   [key_len: u32] [key_bytes] [bitmap_len: u32] [bitmap_data]  × N
    /// [series_count: u32]
    ///   [series_id: u64] [tags_count: u32]
    ///     [k_len: u32] [k_bytes] [v_len: u32] [v_bytes]  × M  × N
    /// ```
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(self.postings.len() as u32).to_le_bytes());
        for (key, bitmap) in &self.postings {
            let key_bytes = key.as_bytes();
            buf.extend_from_slice(&(key_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(key_bytes);
            let mut bm_data = Vec::new();
            bitmap.serialize_into(&mut bm_data).unwrap_or(());
            buf.extend_from_slice(&(bm_data.len() as u32).to_le_bytes());
            buf.extend_from_slice(&bm_data);
        }

        buf.extend_from_slice(&(self.series_tags.len() as u32).to_le_bytes());
        for (&id, tags) in &self.series_tags {
            buf.extend_from_slice(&id.to_le_bytes());
            buf.extend_from_slice(&(tags.len() as u32).to_le_bytes());
            for (k, v) in tags {
                buf.extend_from_slice(&(k.len() as u32).to_le_bytes());
                buf.extend_from_slice(k.as_bytes());
                buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
                buf.extend_from_slice(v.as_bytes());
            }
        }
        buf
    }

    /// 从二进制数据反序列化重建倒排索引
    ///
    /// 是 `serialize()` 的逆操作。任何一步解析失败均返回 None。
    ///
    /// # 参数
    /// - `data`: serialize() 输出的二进制数据
    ///
    /// # 返回
    /// - `Some(InvertedIndex)`: 成功还原的索引实例
    /// - `None`: 数据格式损坏或不完整
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        let mut offset = 0;
        let postings_count = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        offset += 4;

        let mut postings = HashMap::new();
        for _ in 0..postings_count {
            let key_len = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
            offset += 4;
            let key = String::from_utf8_lossy(&data[offset..offset + key_len]).to_string();
            offset += key_len;
            let bm_len = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
            offset += 4;
            let bitmap = RoaringBitmap::deserialize_from(&data[offset..offset + bm_len]).ok()?;
            offset += bm_len;
            postings.insert(key, bitmap);
        }

        let series_count = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        offset += 4;

        let mut series_tags = HashMap::new();
        for _ in 0..series_count {
            let id = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            offset += 8;
            let tags_count = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
            offset += 4;
            let mut tags = Vec::with_capacity(tags_count);
            for _ in 0..tags_count {
                let k_len = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
                offset += 4;
                let k = String::from_utf8_lossy(&data[offset..offset + k_len]).to_string();
                offset += k_len;
                let v_len = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
                offset += 4;
                let v = String::from_utf8_lossy(&data[offset..offset + v_len]).to_string();
                offset += v_len;
                tags.push((k, v));
            }
            series_tags.insert(id, tags);
        }

        Some(Self { postings, series_tags })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_query() {
        let mut idx = InvertedIndex::new();
        idx.add_series(1, &[
            ("host".to_string(), "server01".to_string()),
            ("region".to_string(), "us-west".to_string()),
        ]);
        idx.add_series(2, &[
            ("host".to_string(), "server02".to_string()),
            ("region".to_string(), "us-west".to_string()),
        ]);
        idx.add_series(3, &[
            ("host".to_string(), "server01".to_string()),
            ("region".to_string(), "eu-central".to_string()),
        ]);

        let result = idx.query_exact("host", "server01");
        assert!(result.contains(1));
        assert!(!result.contains(2));
        assert!(result.contains(3));
    }

    #[test]
    fn test_intersection() {
        let mut idx = InvertedIndex::new();
        idx.add_series(1, &[
            ("host".to_string(), "server01".to_string()),
            ("region".to_string(), "us-west".to_string()),
        ]);
        idx.add_series(2, &[
            ("host".to_string(), "server01".to_string()),
            ("region".to_string(), "eu-central".to_string()),
        ]);

        let result = idx.query_intersection(&[
            ("host".to_string(), "server01".to_string()),
            ("region".to_string(), "us-west".to_string()),
        ]);

        assert!(result.contains(1));
        assert!(!result.contains(2));
    }

    #[test]
    fn test_remove_series() {
        let mut idx = InvertedIndex::new();
        idx.add_series(1, &[("host".to_string(), "server01".to_string())]);
        idx.remove_series(1);

        let result = idx.query_exact("host", "server01");
        assert!(result.is_empty());
    }
}
