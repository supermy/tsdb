//! # 索引管理器（Index Manager）— 统一索引调度
//!
//! ## 架构设计
//!
//! IndexManager 是 TSDB 索引子系统的顶层协调者，管理两类索引：
//!
//! ```text
//! IndexManager
//! ├── time_index: HashMap<measurement, SkipList>
//! │   └── 时间戳 → 数据块偏移量（支持范围查询）
//! │
//! ├── tag_index: HashMap<measurement, InvertedIndex>
//! │   └── Tag 键值对 → SeriesId 集合（支持标签过滤）
//! │
//! ├── series_cache: HashMap<series_key, SeriesId>
//! │   └── (measurement,tags) → 唯一 ID（去重）
//! │
//! └── next_series_id: SeriesId (自增计数器)
//! ```
//!
//! ## 索引流程
//!
//! ```text
//! DataPoint 写入
//!      │
//!      ▼
//! 生成 series_key = "cpu,host=server01,region=us-west"
//!      │
//!      ├─ 首次见到？ → 分配新 SeriesId → 更新 tag_index (InvertedIndex)
//!      │
//!      ▼
//!  插入 time_index (SkipList): timestamp → block_offset
//! ```
//!

use crate::skiplist::SkipList;
use crate::inverted::InvertedIndex;
use tsdb_types::model::SeriesId;
use std::collections::HashMap;

/// 索引管理器 — 统一管理时间索引和标签索引
///
/// 为每个 measurement 维护独立的 SkipList（时间索引）和 InvertedIndex（标签索引），
/// 同时通过 series_cache 保证同一 (measurement, tags) 组合始终映射到同一 SeriesId。
///
/// ## 持久化支持
///
/// 通过 `serialize_all()` / `deserialize_entry()` 支持将所有索引数据
/// 序列化到 RocksDB 的 metadata 列族中，实现服务重启后的索引恢复。
pub struct IndexManager {
    /// 时间索引：measurement 名称 → SkipList（时间戳→块偏移量的有序映射）
    time_index: HashMap<String, SkipList>,
    /// 标签索引：measurement 名称 → InvertedIndex（Tag→SeriesId 的倒排映射）
    tag_index: HashMap<String, InvertedIndex>,
    /// 下一个可分配的 SeriesId（全局自增，从 1 开始）
    next_series_id: SeriesId,
    /// 序列键缓存：(measurement + tags) → SeriesId，避免重复创建序列
    series_cache: HashMap<String, SeriesId>,
}

impl IndexManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn index_data_point(
        &mut self,
        measurement: &str,
        tags: &std::collections::BTreeMap<String, String>,
        timestamp: i64,
        block_offset: u64,
    ) -> SeriesId {
        let series_key = format!("{},{}", measurement,
            tags.iter().map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<_>>().join(","));

        let series_id = *self.series_cache
            .entry(series_key.clone())
            .or_insert_with(|| {
                let id = self.next_series_id;
                self.next_series_id += 1;

                let tag_index = self.tag_index
                    .entry(measurement.to_string())
                    .or_default();
                let tag_pairs: Vec<(String, String)> = tags.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                tag_index.add_series(id, &tag_pairs);

                id
            });

        let time_index = self.time_index
            .entry(measurement.to_string())
            .or_insert_with(|| SkipList::new(16));
        time_index.insert(timestamp, block_offset);

        series_id
    }

    /// 按时间范围查询数据块偏移量
    ///
    /// 利用 SkipList 的范围查询能力快速定位 [start, end] 时间窗口内的所有数据块。
    ///
    /// # 参数
    /// - `measurement`: 指标名称
    /// - `start`: 起始时间戳（微秒，包含）
    /// - `end`: 结束时间戳（微秒，包含）
    ///
    /// # 返回
    /// 匹配的时间戳及其对应的数据块偏移量列表
    pub fn query_by_time_range(
        &self,
        measurement: &str,
        start: i64,
        end: i64,
    ) -> Vec<(i64, Vec<u64>)> {
        if let Some(time_idx) = self.time_index.get(measurement) {
            time_idx.range_query(start, end)
        } else {
            Vec::new()
        }
    }

    /// 按 Tag 条件查询匹配的序列 ID 集合
    ///
    /// 委托给指定 measurement 的 InvertedIndex 执行交集查询（AND 语义）。
    ///
    /// # 参数
    /// - `measurement`: 指标名称
    /// - `filters`: 标签过滤条件列表（AND 关系）
    ///
    /// # 返回
    /// 同时满足所有条件的 SeriesId 集合（RoaringBitmap）
    pub fn query_by_tags(
        &self,
        measurement: &str,
        filters: &[(String, String)],
    ) -> roaring::RoaringBitmap {
        if let Some(tag_idx) = self.tag_index.get(measurement) {
            tag_idx.query_intersection(filters)
        } else {
            roaring::RoaringBitmap::new()
        }
    }

    /// 根据 series_key 查找对应的 SeriesId
    ///
    /// # 参数
    /// - `series_key`: 由 `index_data_point` 内部生成的序列标识字符串
    ///
    /// # 返回
    /// - `Some(SeriesId)`: 找到对应 ID
    /// - `None`: 该序列未被索引过
    pub fn get_series_id(&self, series_key: &str) -> Option<SeriesId> {
        self.series_cache.get(series_key).copied()
    }

    /// 获取指定 measurement 下已索引的序列数量
    pub fn series_count(&self, measurement: &str) -> usize {
        self.tag_index.get(measurement)
            .map(|idx| idx.series_count())
            .unwrap_or(0)
    }

    /// 将所有索引数据序列化为 key-value 对（用于批量持久化）
    ///
    /// 输出格式：
    /// ```text
    /// "index:time:<measurement>"     → SkipList 二进制数据
    /// "index:tag:<measurement>"      → InvertedIndex 二进制数据
    /// "index:meta:next_series_id"    → u64 LE 字节
    /// ```
    ///
    /// # 返回
    /// key-value 映射表，可直接逐条写入 RocksDB metadata CF
    pub fn serialize_all(&self) -> HashMap<String, Vec<u8>> {
        let mut result = HashMap::new();
        for (measurement, sl) in &self.time_index {
            let key = format!("index:time:{}", measurement);
            result.insert(key, sl.serialize());
        }
        for (measurement, idx) in &self.tag_index {
            let key = format!("index:tag:{}", measurement);
            result.insert(key, idx.serialize());
        }
        result.insert("index:meta:next_series_id".to_string(), self.next_series_id.to_le_bytes().to_vec());
        result
    }

    /// 从单条持久化数据恢复索引条目（启动时调用）
    ///
    /// 根据 key 前缀判断数据类型并分发到对应的反序列化逻辑：
    /// - `"index:time:*"` → SkipList::deserialize() → time_index
    /// - `"index:tag:*"` → InvertedIndex::deserialize() → tag_index
    /// - `"index:meta:next_series_id"` → next_series_id
    ///
    /// # 参数
    /// - `key`: RocksDB 中的索引 key
    /// - `data`: 对应的二进制 value
    ///
    /// # 返回
    /// - `true`: 成功反序列化并插入索引
    /// - `false`: key 格式不匹配或数据损坏
    pub fn deserialize_entry(&mut self, key: &str, data: &[u8]) -> bool {
        if let Some(measurement) = key.strip_prefix("index:time:") {
            if let Some(sl) = SkipList::deserialize(data) {
                self.time_index.insert(measurement.to_string(), sl);
                return true;
            }
        } else if let Some(measurement) = key.strip_prefix("index:tag:") {
            if let Some(idx) = InvertedIndex::deserialize(data) {
                self.tag_index.insert(measurement.to_string(), idx);
                return true;
            }
        } else if key == "index:meta:next_series_id" && data.len() >= 8 {
            self.next_series_id = u64::from_le_bytes(data[0..8].try_into().unwrap_or([0; 8]));
            return true;
        }
        false
    }
}

impl Default for IndexManager {
    fn default() -> Self {
        Self {
            time_index: HashMap::new(),
            tag_index: HashMap::new(),
            next_series_id: 1,
            series_cache: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_data_point() {
        let mut mgr = IndexManager::new();
        let mut tags = std::collections::BTreeMap::new();
        tags.insert("host".to_string(), "server01".to_string());

        let sid = mgr.index_data_point("cpu", &tags, 1_000_000_000, 0);
        assert_eq!(sid, 1);

        let sid2 = mgr.index_data_point("cpu", &tags, 1_000_030_000, 1);
        assert_eq!(sid2, 1);

        let results = mgr.query_by_time_range("cpu", 0, 2_000_000_000);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_tag_query() {
        let mut mgr = IndexManager::new();
        let mut tags1 = std::collections::BTreeMap::new();
        tags1.insert("host".to_string(), "server01".to_string());
        tags1.insert("region".to_string(), "us-west".to_string());

        let mut tags2 = std::collections::BTreeMap::new();
        tags2.insert("host".to_string(), "server02".to_string());
        tags2.insert("region".to_string(), "us-west".to_string());

        mgr.index_data_point("cpu", &tags1, 1_000_000_000, 0);
        mgr.index_data_point("cpu", &tags2, 1_000_000_000, 1);

        let result = mgr.query_by_tags("cpu", &[
            ("region".to_string(), "us-west".to_string()),
        ]);
        assert_eq!(result.len(), 2);

        let result = mgr.query_by_tags("cpu", &[
            ("host".to_string(), "server01".to_string()),
        ]);
        assert_eq!(result.len(), 1);
    }
}
