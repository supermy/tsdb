//! 错误类型定义模块 - Error Type Definition Module
//!
//! 本模块定义了 TSDB 的统一错误类型 `TsdbError`，
//! 替代散落的 `anyhow::Result`，提供更好的错误追踪和处理。
//!
//! ## 错误分类
//!
//! | 类别 | 变体 | 描述 |
//! |------|------|------|
//! | 存储 | Storage, ColumnFamilyNotFound | RocksDB 相关错误 |
//! | 数据 | InvalidDataPoint | 数据格式错误 |
//! | 压缩 | Compression, Decompression | 压缩/解压错误 |
//! | 索引 | Index | 索引操作错误 |
//! | 查询 | Query | SQL 解析和执行错误 |
//! | 网络 | Network, Protocol, Nng | 网络通信错误 |
//! | 其他 | Config, Plugin, Dashboard, NotFound, Internal | 其他错误 |

use thiserror::Error;

/// TSDB 统一错误类型
///
/// 使用 `thiserror` 派生，实现 `std::error::Error` trait。
/// 所有变体都实现了 `Display`，可以转换为用户友好的错误消息。
///
/// # 使用示例
///
/// ```rust
/// use tsdb_core::error::{TsdbError, Result};
///
/// fn read_data() -> Result<Vec<u8>> {
///     let data = std::fs::read("data.bin")
///         .map_err(|e| TsdbError::Storage(format!("read failed: {}", e)))?;
///     Ok(data)
/// }
/// ```
#[derive(Error, Debug)]
pub enum TsdbError {
    /// 存储引擎错误
    ///
    /// RocksDB 操作失败，如写入、读取、Compaction 等。
    #[error("storage error: {0}")]
    Storage(String),

    /// 无效数据点错误
    ///
    /// 数据点格式不正确，如缺少必要字段、时间戳无效等。
    #[error("invalid data point: {0}")]
    InvalidDataPoint(String),

    /// ColumnFamily 未找到错误
    ///
    /// 尝试访问不存在的 ColumnFamily。
    /// 通常发生在访问已过期的日期 CF 时。
    #[error("column family not found: {0}")]
    ColumnFamilyNotFound(String),

    /// 压缩错误
    ///
    /// 数据压缩失败，如 Delta/Gorilla 编码错误。
    #[error("compression error: {0}")]
    Compression(String),

    /// 解压错误
    ///
    /// 数据解压失败，如数据损坏、格式不匹配等。
    #[error("decompression error: {0}")]
    Decompression(String),

    /// 索引错误
    ///
    /// 索引操作失败，如 SkipList/InvertedIndex 操作错误。
    #[error("index error: {0}")]
    Index(String),

    /// 查询错误
    ///
    /// SQL 解析或执行失败，如语法错误、表不存在等。
    #[error("query error: {0}")]
    Query(String),

    /// 配置错误
    ///
    /// 配置加载或解析失败，如配置文件格式错误、参数无效等。
    #[error("config error: {0}")]
    Config(String),

    /// I/O 错误
    ///
    /// 文件系统操作失败，自动从 `std::io::Error` 转换。
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// RocksDB 错误
    ///
    /// RocksDB 内部错误，自动从 `rocksdb::Error` 转换。
    #[error("rocksdb error: {0}")]
    RocksDb(#[from] rocksdb::Error),

    /// 序列化错误
    ///
    /// JSON/MessagePack 序列化或反序列化失败。
    #[error("serialization error: {0}")]
    Serialization(String),

    /// 网络错误
    ///
    /// 网络连接失败，如 TCP 绑定失败、连接超时等。
    #[error("network error: {0}")]
    Network(String),

    /// 协议错误
    ///
    /// 协议解析失败，如无效的请求格式、版本不匹配等。
    #[error("protocol error: {0}")]
    Protocol(String),

    /// 插件错误
    ///
    /// 插件加载或执行失败。
    #[error("plugin error: {0}")]
    Plugin(String),

    /// NNG 错误
    ///
    /// NNG 消息传递操作失败。
    #[error("nng error: {0}")]
    Nng(String),

    /// 仪表盘错误
    ///
    /// 仪表盘渲染或数据处理失败。
    #[error("dashboard error: {0}")]
    Dashboard(String),

    /// 未找到错误
    ///
    /// 请求的资源不存在，如数据库、表、数据点等。
    #[error("not found: {0}")]
    NotFound(String),

    /// 内部错误
    ///
    /// 内部逻辑错误，如不变量违反、断言失败等。
    /// 通常表示代码 bug，需要开发者关注。
    #[error("internal error: {0}")]
    Internal(String),
}

/// 从 serde_json::Error 自动转换为 TsdbError
///
/// 允许直接使用 `?` 操作符处理 JSON 序列化错误。
impl From<serde_json::Error> for TsdbError {
    fn from(e: serde_json::Error) -> Self {
        TsdbError::Serialization(e.to_string())
    }
}

impl From<bincode::Error> for TsdbError {
    fn from(e: bincode::Error) -> Self {
        TsdbError::Serialization(e.to_string())
    }
}

impl From<rmp_serde::decode::Error> for TsdbError {
    fn from(e: rmp_serde::decode::Error) -> Self {
        TsdbError::Serialization(e.to_string())
    }
}

impl From<rmp_serde::encode::Error> for TsdbError {
    fn from(e: rmp_serde::encode::Error) -> Self {
        TsdbError::Serialization(e.to_string())
    }
}

/// TSDB 结果类型别名
///
/// 使用 `Result<T>` 替代 `std::result::Result<T, TsdbError>`，
/// 简化函数签名。
///
/// # 示例
///
/// ```rust
/// use tsdb_core::error::Result;
///
/// fn process() -> Result<()> {
///     // 操作...
///     Ok(())
/// }
/// ```
pub type Result<T> = std::result::Result<T, TsdbError>;
