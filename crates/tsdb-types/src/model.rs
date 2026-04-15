//! 数据模型定义模块 - Data Model Definition Module
//!
//! 本模块定义了 TSDB 的核心数据结构：
//! - `DataPoint`: 数据点，时序数据的基本单位
//! - `FieldValue`: 字段值，支持多种数据类型
//! - `Tags`: 标签集合，用于标识时间序列
//! - `Fields`: 字段集合，存储实际数据
//!
//! ## 数据模型层次
//!
//! ```text
//! Measurement (指标)
//!   └── Series (时间序列) = Measurement + Tags
//!         └── DataPoint (数据点) = Series + Timestamp + Fields
//! ```
//!
//! ## 示例
//!
//! ```rust
//! use tsdb_types::model::{DataPoint, FieldValue};
//!
//! let dp = DataPoint::new("cpu", 1704067200_000_000)
//!     .with_tag("host", "server01")
//!     .with_tag("region", "us-west")
//!     .with_field("usage", FieldValue::Float(0.75))
//!     .with_field("system", FieldValue::Float(0.25));
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 时间序列 ID 类型
///
/// 用于唯一标识一个时间序列（Measurement + Tags 组合）
pub type SeriesId = u64;

/// 时间戳类型（微秒精度）
///
/// 使用 i64 存储微秒级时间戳：
/// - 范围：约 ±292,000 年
/// - 精度：1 微秒 = 1,000 纳秒 = 0.000001 秒
pub type Timestamp = i64;

/// 标签集合类型
///
/// 使用 BTreeMap 存储，保证：
/// - 有序迭代：序列化结果一致
/// - 快速查找：O(log n) 复杂度
/// - 去重：相同 key 自动覆盖
pub type Tags = BTreeMap<String, String>;

/// 字段集合类型
///
/// 每个数据点可包含多个命名字段，如：
/// - cpu 数据点：usage, system, idle, iowait
/// - memory 数据点：used, free, cached, buffers
pub type Fields = BTreeMap<String, FieldValue>;

/// 字段值枚举 - Field Value Enum
///
/// 支持四种数据类型，覆盖常见时序数据场景：
///
/// | 类型 | 适用场景 | 示例 |
/// |------|----------|------|
/// | Float | 温度、CPU 使用率、网络流量 | 0.75, 98.6, 1024.5 |
/// | Integer | 请求计数、错误数、连接数 | 100, 0, 42 |
/// | String | 主机名、状态码、版本号 | "server01", "200", "v1.0.0" |
/// | Boolean | 开关状态、健康检查 | true, false |
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FieldValue {
    /// 浮点数值（64 位双精度）
    Float(f64),
    /// 整数值（64 位有符号）
    Integer(i64),
    /// 字符串值（UTF-8）
    String(String),
    /// 布尔值
    Boolean(bool),
}

impl FieldValue {
    /// 尝试转换为 f64
    ///
    /// # 返回值
    ///
    /// - Float: 直接返回值
    /// - Integer: 转换为 f64（可能丢失精度）
    /// - 其他: 返回 None
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            FieldValue::Float(v) => Some(*v),
            FieldValue::Integer(v) => Some(*v as f64),
            _ => None,
        }
    }

    /// 尝试转换为 i64
    ///
    /// # 返回值
    ///
    /// - Integer: 直接返回值
    /// - Float: 截断为 i64（丢失小数部分）
    /// - 其他: 返回 None
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            FieldValue::Integer(v) => Some(*v),
            FieldValue::Float(v) => Some(*v as i64),
            _ => None,
        }
    }

    /// 尝试获取字符串引用
    ///
    /// # 返回值
    ///
    /// - String: 返回字符串切片
    /// - 其他: 返回 None
    pub fn as_str(&self) -> Option<&str> {
        match self {
            FieldValue::String(v) => Some(v),
            _ => None,
        }
    }

    /// 尝试转换为 bool
    ///
    /// # 返回值
    ///
    /// - Boolean: 直接返回值
    /// - 其他: 返回 None
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            FieldValue::Boolean(v) => Some(*v),
            _ => None,
        }
    }
}

/// 数据点 - Data Point
///
/// 时序数据的基本单位，包含：
/// - `measurement`: 指标名称（如 "cpu", "memory"）
/// - `tags`: 标签集合（如 {"host": "server01", "region": "us-west"}）
/// - `fields`: 字段集合（如 {"usage": 0.75, "system": 0.25}）
/// - `timestamp`: 时间戳（微秒精度）
///
/// ## 设计原则
///
/// 1. **不可变语义**: 创建后不应修改（推荐使用 builder 模式）
/// 2. **时间精度**: 微秒级，满足高频采集场景
/// 3. **类型安全**: 字段值强类型，避免运行时错误
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataPoint {
    /// 指标名称
    pub measurement: String,
    /// 标签集合
    pub tags: Tags,
    /// 字段集合
    pub fields: Fields,
    /// 时间戳（微秒）
    pub timestamp: Timestamp,
}

impl DataPoint {
    /// 创建新的数据点
    ///
    /// # 参数
    ///
    /// - `measurement`: 指标名称
    /// - `timestamp`: 时间戳（微秒）
    ///
    /// # 返回值
    ///
    /// 空的数据点（无标签、无字段）
    ///
    /// # 示例
    ///
    /// ```rust
    /// let dp = DataPoint::new("cpu", 1704067200_000_000);
    /// ```
    pub fn new(measurement: impl Into<String>, timestamp: Timestamp) -> Self {
        Self {
            measurement: measurement.into(),
            tags: Tags::new(),
            fields: Fields::new(),
            timestamp,
        }
    }

    /// 添加标签（Builder 模式）
    ///
    /// # 参数
    ///
    /// - `key`: 标签键
    /// - `value`: 标签值
    ///
    /// # 返回值
    ///
    /// 修改后的数据点（支持链式调用）
    ///
    /// # 示例
    ///
    /// ```rust
    /// let dp = DataPoint::new("cpu", 0)
    ///     .with_tag("host", "server01")
    ///     .with_tag("region", "us-west");
    /// ```
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }

    /// 添加字段（Builder 模式）
    ///
    /// # 参数
    ///
    /// - `key`: 字段名
    /// - `value`: 字段值
    ///
    /// # 返回值
    ///
    /// 修改后的数据点（支持链式调用）
    ///
    /// # 示例
    ///
    /// ```rust
    /// let dp = DataPoint::new("cpu", 0)
    ///     .with_field("usage", FieldValue::Float(0.75))
    ///     .with_field("count", FieldValue::Integer(100));
    /// ```
    pub fn with_field(mut self, key: impl Into<String>, value: FieldValue) -> Self {
        self.fields.insert(key.into(), value);
        self
    }

    /// 生成序列键
    ///
    /// 序列键用于标识唯一的时间序列，格式：
    /// `measurement,tag1=value1,tag2=value2,...`
    ///
    /// 标签按字母顺序排序，保证相同标签集合生成相同的键。
    ///
    /// # 返回值
    ///
    /// 序列键字符串
    pub fn series_key(&self) -> String {
        let mut parts: Vec<String> = self.tags.iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        parts.sort();  // 排序保证一致性
        format!("{},{}", self.measurement, parts.join(","))
    }
}

/// 指标元数据 - Measurement Metadata
///
/// 描述一个指标的结构信息，包括：
/// - 名称
/// - 标签键列表
/// - 字段键列表
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measurement {
    /// 指标名称
    pub name: String,
    /// 标签键列表
    pub tag_keys: Vec<String>,
    /// 字段键列表
    pub field_keys: Vec<String>,
}

/// 字段类型枚举 - Field Type Enum
///
/// 用于描述字段的数据类型，不包含具体值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    /// 浮点类型
    Float,
    /// 整数类型
    Integer,
    /// 字符串类型
    String,
    /// 布尔类型
    Boolean,
}

/// 从 FieldValue 推断 FieldType
impl From<&FieldValue> for FieldType {
    fn from(value: &FieldValue) -> Self {
        match value {
            FieldValue::Float(_) => FieldType::Float,
            FieldValue::Integer(_) => FieldType::Integer,
            FieldValue::String(_) => FieldType::String,
            FieldValue::Boolean(_) => FieldType::Boolean,
        }
    }
}
