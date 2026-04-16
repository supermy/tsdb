//! # 维度表（Dimension Table）— 标签字典编码
//!
//! ## 设计动机
//!
//! 时间序列数据中的 Tag（标签）通常是重复度很高的字符串，例如：
//! - `host=server01`, `host=server02`, `region=us-west`
//!
//! 直接存储原始字符串会带来：
//! 1. **空间浪费**：同一标签值在成千上万数据点中重复存储
//! 2. **比较低效**：字符串哈希/比对比整数 ID 慢得多
//!
//! ## 解决方案：字典编码（Dictionary Encoding）
//!
//! 将每个 tag key 和 tag value 映射为紧凑的整数 ID：
//!
//! ```text
//! 原始 Tags: { host: "server01", region: "us-west" }
//!       ↓ encode_tags()
//! 编码结果: [(1, 3), (2, 5)]   ← (key_id, value_id) 对的有序列表
//!       ↓ compute_tag_signature()
//! 签名哈希: 0xA3F7B2C1          ← 用于 RowKey 的 tags_hash 字段
//! ```
//!
//! ## 数据结构
//!
//! | 内存表 | 键类型 | 值类型 | 用途 |
//! |--------|--------|--------|------|
//! | `tag_key_ids` | `String` (tag key) | `u32` (ID) | tag key → ID 映射 |
//! | `tag_value_ids` | `(u32, String)` (key_id, value) | `u32` (ID) | tag value → ID 映射 |
//!

use rocksdb::DB;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// 维度列族名称（用于持久化 tag 映射关系）
#[allow(dead_code)]
const DIMENSION_CF: &str = "dimension";

/// 维度表 — 管理 Tag Key / Tag Value 的字典编码与解码
///
/// 提供双向映射能力：
/// - **编码方向**：`"host"` → `1`, `"server01"` → `3`（写入路径使用）
/// - **解码方向**：`1` → `"host"`, `3` → `"server01"`（查询路径使用）
///
/// ## ID 分配策略
///
/// 使用 `AtomicU64` 自增计数器保证线程安全的 ID 分配，
/// tag key 和 tag value 各自维护独立的 ID 空间。
pub struct DimensionTable {
    /// 底层 RocksDB 实例（预留持久化扩展能力）
    #[allow(dead_code)]
    db: Arc<DB>,
    /// Tag Key → ID 的内存映射表（写时复制语义，Mutex 保护）
    tag_key_ids: std::sync::Mutex<HashMap<String, u32>>,
    /// (Key_ID, Tag_Value) → ID 的内存映射表（复合键确保全局唯一）
    tag_value_ids: std::sync::Mutex<HashMap<(u32, String), u32>>,
    /// 下一个可用的 Tag Key ID（原子自增，从 1 开始）
    next_key_id: AtomicU64,
    /// 下一个可用的 Tag Value ID（原子自增，从 1 开始）
    next_value_id: AtomicU64,
}

impl DimensionTable {
    /// 创建新的维度表实例
    ///
    /// 初始化空的映射表和 ID 计数器。后续可通过 RocksDB 持久化恢复已有映射。
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            db,
            tag_key_ids: std::sync::Mutex::new(HashMap::new()),
            tag_value_ids: std::sync::Mutex::new(HashMap::new()),
            next_key_id: AtomicU64::new(1),
            next_value_id: AtomicU64::new(1),
        }
    }

    /// 对 Tag Key 进行字典编码，返回对应的整型 ID
    ///
    /// 如果该 key 已存在则直接返回已有 ID，否则分配新 ID 并记录映射关系。
    /// 使用 `fetch_add` 保证多线程环境下 ID 分配的唯一性。
    ///
    /// # 参数
    /// - `key`: 原始 tag key 字符串（如 `"host"`, `"region"`）
    ///
    /// # 返回
    /// 该 key 对应的唯一整型 ID（u32）
    pub fn encode_tag_key(&self, key: &str) -> u32 {
        let mut map = self.tag_key_ids.lock().unwrap();
        if let Some(&id) = map.get(key) {
            return id;
        }
        let id = self.next_key_id.fetch_add(1, Ordering::Relaxed) as u32;
        map.insert(key.to_string(), id);
        id
    }

    /// 根据 ID 反查 Tag Key 的原始字符串
    ///
    /// # 参数
    /// - `id`: 通过 `encode_tag_key` 获得的整型 ID
    ///
    /// # 返回
    /// - `Some(String)`: 对应的原始 tag key
    /// - `None`: 该 ID 不存在（可能已被清理或从未分配）
    pub fn decode_tag_key(&self, id: u32) -> Option<String> {
        let map = self.tag_key_ids.lock().unwrap();
        map.iter().find(|(_, &v)| v == id).map(|(k, _)| k.clone())
    }

    /// 对 Tag Value 进行字典编码，返回对应的整型 ID
    ///
    /// value 的编码依赖于其所属 key 的 ID，形成 `(key_id, value)` 复合键，
    /// 确保 不同 key 下同名 value（如 `host=x` vs `region=x`）不会冲突。
    ///
    /// # 参数
    /// - `key_id`: 该 value 所属 tag key 的 ID（通过 `encode_tag_key` 获得）
    /// - `value`: 原始 tag value 字符串（如 `"server01"`, `"us-west"`）
    ///
    /// # 返回
    /// 该 (key, value) 对对应的唯一整型 ID（u32）
    pub fn encode_tag_value(&self, key_id: u32, value: &str) -> u32 {
        let mut map = self.tag_value_ids.lock().unwrap();
        let key = (key_id, value.to_string());
        if let Some(&id) = map.get(&key) {
            return id;
        }
        let id = self.next_value_id.fetch_add(1, Ordering::Relaxed) as u32;
        map.insert(key, id);
        id
    }

    /// 根据 key_id 和 value_id 反查 Tag Value 的原始字符串
    ///
    /// # 参数
    /// - `key_id`: tag key 的 ID
    /// - `value_id`: tag value 的 ID
    ///
    /// # 返回
    /// - `Some(String)`: 对应的原始 tag value
    /// - `None`: 该组合不存在
    pub fn decode_tag_value(&self, key_id: u32, value_id: u32) -> Option<String> {
        let map = self.tag_value_ids.lock().unwrap();
        map.iter()
            .find(|((k, _), &v)| *k == key_id && v == value_id)
            .map(|((_, v), _)| v.clone())
    }

    /// 对完整的 Tags 集合进行批量编码
    ///
    /// 遍历所有 (key, value) 对，分别编码后按 key_id 排序返回。
    /// 排序保证了相同 tags 集合无论插入顺序如何都产生相同的编码结果，
    /// 这是计算一致签名的前提条件。
    ///
    /// # 参数
    /// - `tags`: 待编码的 Tags 映射表
    ///
    /// # 返回
    /// 按 key_id 升序排列的 `[(key_id, value_id)]` 列表
    pub fn encode_tags(&self, tags: &tsdb_types::model::Tags) -> Vec<(u32, u32)> {
        let mut result = Vec::with_capacity(tags.len());
        for (key, value) in tags {
            let key_id = self.encode_tag_key(key);
            let value_id = self.encode_tag_value(key_id, value);
            result.push((key_id, value_id));
        }
        result.sort_by_key(|(k, _)| *k);
        result
    }

    /// 将编码后的 (key_id, value_id) 列表还原为原始 Tags
    ///
    /// 是 `encode_tags` 的逆操作，用于查询结果展示。
    ///
    /// # 参数
    /// - `encoded`: 通过 `encode_tags` 生成的编码列表
    ///
    /// # 返回
    /// 还原后的 `Tags` 映射表
    pub fn decode_tags(&self, encoded: &[(u32, u32)]) -> tsdb_types::model::Tags {
        let mut tags = tsdb_types::model::Tags::new();
        for (key_id, value_id) in encoded {
            if let Some(key) = self.decode_tag_key(*key_id) {
                if let Some(value) = self.decode_tag_value(*key_id, *value_id) {
                    tags.insert(key, value);
                }
            }
        }
        tags
    }

    /// 计算 Tags 集合的确定性签名哈希
    ///
    /// 先对 tags 进行编码（保证排序一致性），再对所有 (key_id, value_id)
    /// 对进行顺序哈希，生成一个 64 位指纹。相同的 tags 集合总是产生相同签名，
    /// 用于 RowKey 中唯一标识一组 tag 组合。
    ///
    /// # 参数
    /// - `tags`: 待计算签名的 Tags 集合
    ///
    /// # 返回
    /// 64 位哈希签名（u64）
    pub fn compute_tag_signature(&self, tags: &tsdb_types::model::Tags) -> u64 {
        let encoded = self.encode_tags(tags);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash, Hasher};
        for (k, v) in &encoded {
            k.hash(&mut hasher);
            v.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// 返回当前已注册的 Tag Key 数量（用于监控和调试）
    pub fn tag_key_count(&self) -> usize {
        self.tag_key_ids.lock().unwrap().len()
    }

    /// 返回当前已注册的 Tag Value 数量（用于监控和调试）
    pub fn tag_value_count(&self) -> usize {
        self.tag_value_ids.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_tag_key() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = Arc::new(DB::open_default(dir.path()).unwrap());
        let dim = DimensionTable::new(db);

        let id1 = dim.encode_tag_key("host");
        let id2 = dim.encode_tag_key("region");
        let id3 = dim.encode_tag_key("host");

        assert_eq!(id1, id3);
        assert_ne!(id1, id2);

        assert_eq!(dim.decode_tag_key(id1), Some("host".to_string()));
        assert_eq!(dim.decode_tag_key(id2), Some("region".to_string()));
    }

    #[test]
    fn test_encode_decode_tags() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = Arc::new(DB::open_default(dir.path()).unwrap());
        let dim = DimensionTable::new(db);

        let mut tags = tsdb_types::model::Tags::new();
        tags.insert("host".to_string(), "server01".to_string());
        tags.insert("region".to_string(), "us-west".to_string());

        let encoded = dim.encode_tags(&tags);
        let decoded = dim.decode_tags(&encoded);

        assert_eq!(decoded, tags);
    }

    #[test]
    fn test_tag_signature() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = Arc::new(DB::open_default(dir.path()).unwrap());
        let dim = DimensionTable::new(db);

        let mut tags1 = tsdb_types::model::Tags::new();
        tags1.insert("host".to_string(), "server01".to_string());

        let mut tags2 = tsdb_types::model::Tags::new();
        tags2.insert("host".to_string(), "server01".to_string());

        assert_eq!(
            dim.compute_tag_signature(&tags1),
            dim.compute_tag_signature(&tags2)
        );
    }
}
