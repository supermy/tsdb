//! RowKey 和 Qualifier 编码模块 - RowKey and Qualifier Encoding Module
//!
//! 本模块实现了 TSDB 的核心数据布局：
//! - `RowKey`: 行键，标识一个时间序列块
//! - `Qualifier`: 列限定符，标识块内的具体字段和时间偏移
//!
//! ## 数据布局设计
//!
//! TSDB 采用 **RowKey + Qualifier** 的二级索引结构：
//!
//! ```text
//! Key:   RowKey | 0x00 | Qualifier
//!        ──────────────────────────
//!        measurement|tags_hash|block_ts | \0 | field_name:micro_offset
//!
//! Value: FieldValue (encoded)
//! ```
//!
//! ## 30 秒定长块设计
//!
//! 时间戳按 30 秒对齐到块边界，优势：
//! 1. 压缩效率：同一块内时间戳增量小，Delta 编码效果好
//! 2. 查询性能：范围查询按块过滤，减少扫描量
//! 3. 冷热分离：按日期分 CF，自然支持数据生命周期管理

use tsdb_types::model::{DataPoint, Timestamp};
use std::hash::{DefaultHasher, Hasher, Hash};

/// 块持续时间（秒）- Block Duration in Seconds
///
/// 30 秒的块大小是经过权衡的选择：
/// - 太小（如 1 秒）：块数量多，元数据开销大
/// - 太大（如 5 分钟）：压缩效率下降，查询延迟增加
pub const BLOCK_DURATION_SECS: u64 = 30;

/// 块持续时间（微秒）- Block Duration in Microseconds
pub const BLOCK_DURATION_MICROS: u64 = BLOCK_DURATION_SECS * 1_000_000;

/// RowKey 字段分隔符 - RowKey Field Separator
///
/// 使用 `|` 分隔 measurement、tags_hash 和 block_timestamp
pub const SEPARATOR: u8 = b'|';

/// Qualifier 字段分隔符 - Qualifier Field Separator
///
/// 使用 `:` 分隔字段名和时间偏移
pub const QUALIFIER_SEPARATOR: u8 = b':';

/// 行键 - Row Key
///
/// 标识一个时间序列块，由三部分组成：
/// - `measurement`: 指标名称（如 "cpu", "memory"）
/// - `tags_hash`: 标签集合的哈希值（用于区分同一指标的不同实例）
/// - `block_start_timestamp`: 块起始时间戳（微秒，对齐到 30 秒边界）
///
/// ## 编码格式
///
/// ```text
/// measurement | tags_hash(8B BE) | block_start_timestamp(8B BE)
/// ```
///
/// ## 示例
///
/// ```rust
/// let dp = DataPoint::new("cpu", 45_000_000)
///     .with_tag("host", "server01");
/// let rk = RowKey::from_data_point(&dp);
/// // rk.measurement = "cpu"
/// // rk.tags_hash = hash({"host": "server01"})
/// // rk.block_start_timestamp = 30_000_000 (对齐到 30 秒边界)
/// ```
#[derive(Debug, Clone)]
pub struct RowKey {
    /// 指标名称
    pub measurement: String,
    /// 标签集合哈希值
    pub tags_hash: u64,
    /// 块起始时间戳（微秒）
    pub block_start_timestamp: Timestamp,
}

impl RowKey {
    /// 从数据点创建 RowKey
    ///
    /// # 参数
    ///
    /// - `dp`: 数据点引用
    ///
    /// # 返回值
    ///
    /// 创建的 RowKey，时间戳已对齐到块边界
    pub fn from_data_point(dp: &DataPoint) -> Self {
        let tags_hash = compute_tags_hash(&dp.tags);
        let block_start = align_to_block_start(dp.timestamp);
        Self {
            measurement: dp.measurement.clone(),
            tags_hash,
            block_start_timestamp: block_start,
        }
    }

    /// 编码为二进制格式
    ///
    /// # 格式
    ///
    /// ```text
    /// [measurement bytes] | [tags_hash:8B BE] | [block_ts:8B BE]
    /// ```
    ///
    /// # 返回值
    ///
    /// 编码后的二进制数据
    pub fn encode(&self) -> Vec<u8> {
        // 预分配容量：measurement + 2个分隔符 + tags_hash(8) + block_ts(8)
        let mut buf = Vec::with_capacity(
            self.measurement.len() + 8 + 8 + 2,
        );

        // 写入 measurement
        buf.extend_from_slice(self.measurement.as_bytes());
        buf.push(SEPARATOR);

        // 写入 tags_hash（大端序，保证字典序正确）
        buf.extend_from_slice(&self.tags_hash.to_be_bytes());
        buf.push(SEPARATOR);

        // 写入 block_start_timestamp（大端序）
        buf.extend_from_slice(&self.block_start_timestamp.to_be_bytes());

        buf
    }

    /// 从二进制格式解码
    ///
    /// # 参数
    ///
    /// - `data`: 二进制数据
    ///
    /// # 返回值
    ///
    /// 解析成功返回 `Some(RowKey)`，格式错误返回 `None`
    pub fn decode(data: &[u8]) -> Option<Self> {
        // 查找第一个分隔符位置
        let sep1 = data.iter().position(|&b| b == SEPARATOR)?;
        let rest = &data[sep1 + 1..];

        // 查找第二个分隔符位置
        let sep2 = rest.iter().position(|&b| b == SEPARATOR)?;

        // 解析各字段
        let measurement = String::from_utf8_lossy(&data[..sep1]).to_string();
        let tags_hash = u64::from_be_bytes(rest[..sep2].try_into().ok()?);
        let block_start_timestamp = i64::from_be_bytes(rest[sep2 + 1..].try_into().ok()?);

        Some(Self {
            measurement,
            tags_hash,
            block_start_timestamp,
        })
    }
}

/// 列限定符 - Column Qualifier
///
/// 标识块内的具体字段和时间偏移，由两部分组成：
/// - `field_name`: 字段名称（如 "usage", "system"）
/// - `microsecond_offset`: 相对于块起始时间戳的微秒偏移量
///
/// ## 编码格式
///
/// ```text
/// field_name : microsecond_offset(4B BE)
/// ```
///
/// ## 示例
///
/// ```rust
/// // 时间戳 45_000_123 微秒，块起始 30_000_000 微秒
/// let q = Qualifier::new("usage", 45_000_123, 30_000_000);
/// // q.field_name = "usage"
/// // q.microsecond_offset = 15_000_123 (45_000_123 - 30_000_000)
/// ```
#[derive(Debug, Clone)]
pub struct Qualifier {
    /// 字段名称
    pub field_name: String,
    /// 微秒偏移量（相对于块起始时间戳）
    pub microsecond_offset: u32,
}

impl Qualifier {
    /// 创建 Qualifier
    ///
    /// # 参数
    ///
    /// - `field_name`: 字段名称
    /// - `timestamp`: 绝对时间戳（微秒）
    /// - `block_start`: 块起始时间戳（微秒）
    ///
    /// # 返回值
    ///
    /// 创建的 Qualifier，microsecond_offset = timestamp - block_start
    pub fn new(field_name: impl Into<String>, timestamp: Timestamp, block_start: Timestamp) -> Self {
        let offset_micros = (timestamp - block_start) as u64;
        Self {
            field_name: field_name.into(),
            microsecond_offset: offset_micros as u32,
        }
    }

    /// 编码为二进制格式
    ///
    /// # 格式
    ///
    /// ```text
    /// [field_name bytes] : [microsecond_offset:4B BE]
    /// ```
    ///
    /// # 返回值
    ///
    /// 编码后的二进制数据
    pub fn encode(&self) -> Vec<u8> {
        // 预分配容量：field_name + 分隔符 + offset(4)
        let mut buf = Vec::with_capacity(self.field_name.len() + 4 + 1);

        // 写入字段名
        buf.extend_from_slice(self.field_name.as_bytes());
        buf.push(QUALIFIER_SEPARATOR);

        // 写入微秒偏移量（大端序）
        buf.extend_from_slice(&self.microsecond_offset.to_be_bytes());

        buf
    }

    /// 从二进制格式解码
    ///
    /// # 参数
    ///
    /// - `data`: 二进制数据
    ///
    /// # 返回值
    ///
    /// 解析成功返回 `Some(Qualifier)`，格式错误返回 `None`
    pub fn decode(data: &[u8]) -> Option<Self> {
        // 查找分隔符位置
        let sep = data.iter().position(|&b| b == QUALIFIER_SEPARATOR)?;

        // 解析各字段
        let field_name = String::from_utf8_lossy(&data[..sep]).to_string();
        let microsecond_offset = u32::from_be_bytes(data[sep + 1..].try_into().ok()?);

        Some(Self {
            field_name,
            microsecond_offset,
        })
    }
}

/// 对齐时间戳到块边界
///
/// 将任意时间戳向下取整到最近的 30 秒边界。
///
/// # 参数
///
/// - `timestamp_micros`: 时间戳（微秒）
///
/// # 返回值
///
/// 对齐后的块起始时间戳（微秒）
///
/// # 示例
///
/// ```rust
/// align_to_block_start(0)           // → 0
/// align_to_block_start(15_000_000)  // → 0 (15秒，仍在第一个块内)
/// align_to_block_start(30_000_000)  // → 30_000_000 (30秒，第二个块起始)
/// align_to_block_start(45_000_000)  // → 30_000_000 (45秒，仍在第二个块内)
/// ```
pub fn align_to_block_start(timestamp_micros: Timestamp) -> Timestamp {
    let block_micros = BLOCK_DURATION_MICROS as Timestamp;
    (timestamp_micros / block_micros) * block_micros
}

/// 计算标签集合的哈希值
///
/// 使用 `DefaultHasher` 计算标签集合的一致性哈希。
/// 由于 `Tags` 是 `BTreeMap`，迭代顺序是确定的，因此哈希值也是确定的。
///
/// # 参数
///
/// - `tags`: 标签集合引用
///
/// # 返回值
///
/// 64 位哈希值
///
/// # 注意
///
/// 此哈希用于区分同一 measurement 的不同时间序列实例，
/// 不应用于安全敏感场景（如密码哈希）。
pub fn compute_tags_hash(tags: &tsdb_types::model::Tags) -> u64 {
    let mut hasher = DefaultHasher::new();

    // BTreeMap 保证迭代顺序一致，因此哈希值确定
    for (k, v) in tags {
        Hash::hash(&k, &mut hasher);
        Hash::hash(&v, &mut hasher);
    }

    hasher.finish()
}

/// 将时间戳转换为 ColumnFamily 名称
///
/// 根据时间戳确定数据应存储的 ColumnFamily。
/// CF 名称格式为 `data_YYYYMMDD`，按自然日划分。
///
/// # 参数
///
/// - `timestamp_micros`: 时间戳（微秒）
///
/// # 返回值
///
/// ColumnFamily 名称字符串
///
/// # 示例
///
/// ```rust
/// let ts = 1704067200_000_000i64;  // 2024-01-01 00:00:00 UTC
/// let name = timestamp_to_cf_name(ts);
/// // name = "data_20240101"
/// ```
pub fn timestamp_to_cf_name(timestamp_micros: Timestamp) -> String {
    // 微秒转秒
    let secs = timestamp_micros / 1_000_000;

    // 秒转日期时间
    let dt = chrono::DateTime::from_timestamp(secs, 0).unwrap_or_default();

    // 格式化为 YYYYMMDD
    format!("data_{}", dt.format("%Y%m%d"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsdb_types::model::FieldValue;
    use std::collections::BTreeMap;

    /// 测试块对齐
    #[test]
    fn test_block_alignment() {
        assert_eq!(align_to_block_start(0), 0);
        assert_eq!(align_to_block_start(15_000_000), 0);
        assert_eq!(align_to_block_start(30_000_000), 30_000_000);
        assert_eq!(align_to_block_start(45_000_000), 30_000_000);
        assert_eq!(align_to_block_start(60_000_000), 60_000_000);
    }

    /// 测试 RowKey 编解码
    #[test]
    fn test_rowkey_encode_decode() {
        let dp = DataPoint::new("cpu", 45_000_000)
            .with_tag("host", "server01")
            .with_tag("region", "us-west");
        let rk = RowKey::from_data_point(&dp);
        let encoded = rk.encode();
        let decoded = RowKey::decode(&encoded).unwrap();
        assert_eq!(decoded.measurement, "cpu");
        assert_eq!(decoded.tags_hash, rk.tags_hash);
        assert_eq!(decoded.block_start_timestamp, 30_000_000);
    }

    /// 测试 Qualifier 编解码
    #[test]
    fn test_qualifier_encode_decode() {
        let q = Qualifier::new("usage", 45_000_123, 30_000_000);
        let encoded = q.encode();
        let decoded = Qualifier::decode(&encoded).unwrap();
        assert_eq!(decoded.field_name, "usage");
        assert_eq!(decoded.microsecond_offset, 15_000_123);
    }

    /// 测试标签哈希的确定性
    #[test]
    fn test_tags_hash_deterministic() {
        // 不同插入顺序的 BTreeMap
        let mut tags1 = BTreeMap::new();
        tags1.insert("host".to_string(), "server01".to_string());
        tags1.insert("region".to_string(), "us-west".to_string());

        let mut tags2 = BTreeMap::new();
        tags2.insert("region".to_string(), "us-west".to_string());
        tags2.insert("host".to_string(), "server01".to_string());

        // 哈希值应该相同
        assert_eq!(compute_tags_hash(&tags1), compute_tags_hash(&tags2));
    }

    /// 测试 CF 名称生成
    #[test]
    fn test_cf_name() {
        let ts = 1704067200_000_000i64;
        let name = timestamp_to_cf_name(ts);
        assert!(name.starts_with("data_"));
    }
}
