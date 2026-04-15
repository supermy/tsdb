//! 协议定义模块 - Protocol Definition Module
//!
//! 本模块定义了 TSDB 客户端和服务端之间的通信协议：
//! - `Request`: 客户端请求枚举
//! - `Response`: 服务端响应枚举
//! - `FieldValueProto`: 字段值的序列化格式
//!
//! ## 协议格式
//!
//! 使用 MessagePack (rmp-serde) 进行二进制序列化：
//! - 紧凑：比 JSON 小约 50%
//! - 快速：序列化/反序列化性能高
//! - 类型安全：保留类型信息
//!
//! ## 消息格式
//!
//! ```text
//! +--------+--------+----------------+
//! | Length | CRC32  | MessagePack    |
//! | (4B)   | (4B)   | (variable)     |
//! +--------+--------+----------------+
//! ```
//!
//! 注意：当前实现未包含 CRC32 校验，后续可添加。

use serde::{Deserialize, Serialize};

/// 客户端请求枚举 - Client Request Enum
///
/// 定义了客户端可以向服务端发送的所有请求类型。
///
/// # 变体
///
/// - `Write`: 写入数据点
/// - `Query`: 执行 SQL 查询
/// - `CreateDatabase`: 创建数据库
/// - `ListDatabases`: 列出所有数据库
/// - `DropDatabase`: 删除数据库
/// - `Ping`: 健康检查
#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    /// 写入数据点请求
    ///
    /// # 字段
    ///
    /// - `measurement`: 指标名称
    /// - `tags`: 标签列表 [(key, value), ...]
    /// - `fields`: 字段列表 [(name, value), ...]
    /// - `timestamp`: 时间戳（微秒）
    Write {
        measurement: String,
        tags: Vec<(String, String)>,
        fields: Vec<(String, FieldValueProto)>,
        timestamp: i64,
    },

    /// SQL 查询请求
    ///
    /// # 字段
    ///
    /// - `sql`: SQL 查询语句
    Query {
        sql: String,
    },

    /// 创建数据库请求
    ///
    /// # 字段
    ///
    /// - `name`: 数据库名称
    CreateDatabase {
        name: String,
    },

    /// 列出数据库请求
    ///
    /// 无参数，返回所有数据库名称列表
    ListDatabases,

    /// 删除数据库请求
    ///
    /// # 字段
    ///
    /// - `name`: 要删除的数据库名称
    DropDatabase {
        name: String,
    },

    /// 健康检查请求
    ///
    /// 无参数，服务端应返回 `Pong` 响应
    Ping,
}

/// 服务端响应枚举 - Server Response Enum
///
/// 定义了服务端可以返回的所有响应类型。
#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    /// 操作成功响应
    ///
    /// 表示请求操作成功完成，无返回数据
    Ok,

    /// 查询结果响应
    ///
    /// # 字段
    ///
    /// - `columns`: 列名列表
    /// - `rows`: 数据行列表，每行是一个字段值列表
    QueryResult {
        columns: Vec<String>,
        rows: Vec<Vec<FieldValueProto>>,
    },

    /// 数据库列表响应
    ///
    /// # 字段
    ///
    /// - `Vec<String>`: 数据库名称列表
    Databases(Vec<String>),

    /// 错误响应
    ///
    /// # 字段
    ///
    /// - `String`: 错误消息
    Error(String),

    /// Pong 响应
    ///
    /// 对 `Ping` 请求的响应
    Pong,
}

/// 字段值协议格式 - Field Value Protocol Format
///
/// 用于序列化的字段值类型，与 `tsdb_types::model::FieldValue` 对应。
/// 使用独立的类型是为了确保协议稳定性，不受内部类型变化影响。
#[derive(Debug, Serialize, Deserialize)]
pub enum FieldValueProto {
    /// 浮点数值
    Float(f64),
    /// 整数值
    Integer(i64),
    /// 字符串值
    String(String),
    /// 布尔值
    Boolean(bool),
}

/// 从内部 FieldValue 转换为协议格式
impl From<tsdb_types::model::FieldValue> for FieldValueProto {
    fn from(v: tsdb_types::model::FieldValue) -> Self {
        match v {
            tsdb_types::model::FieldValue::Float(f) => FieldValueProto::Float(f),
            tsdb_types::model::FieldValue::Integer(i) => FieldValueProto::Integer(i),
            tsdb_types::model::FieldValue::String(s) => FieldValueProto::String(s),
            tsdb_types::model::FieldValue::Boolean(b) => FieldValueProto::Boolean(b),
        }
    }
}

/// 从协议格式转换为内部 FieldValue
impl From<FieldValueProto> for tsdb_types::model::FieldValue {
    fn from(v: FieldValueProto) -> Self {
        match v {
            FieldValueProto::Float(f) => tsdb_types::model::FieldValue::Float(f),
            FieldValueProto::Integer(i) => tsdb_types::model::FieldValue::Integer(i),
            FieldValueProto::String(s) => tsdb_types::model::FieldValue::String(s),
            FieldValueProto::Boolean(b) => tsdb_types::model::FieldValue::Boolean(b),
        }
    }
}

/// 编码请求为 MessagePack 格式
///
/// # 参数
///
/// - `req`: 请求引用
///
/// # 返回值
///
/// MessagePack 编码后的二进制数据
pub fn encode_request(req: &Request) -> Vec<u8> {
    rmp_serde::to_vec(req).unwrap_or_default()
}

/// 从 MessagePack 格式解码请求
///
/// # 参数
///
/// - `data`: MessagePack 编码的二进制数据
///
/// # 返回值
///
/// 解码成功返回 `Some(Request)`，失败返回 `None`
pub fn decode_request(data: &[u8]) -> Option<Request> {
    rmp_serde::from_slice(data).ok()
}

/// 编码响应为 MessagePack 格式
///
/// # 参数
///
/// - `resp`: 响应引用
///
/// # 返回值
///
/// MessagePack 编码后的二进制数据
pub fn encode_response(resp: &Response) -> Vec<u8> {
    rmp_serde::to_vec(resp).unwrap_or_default()
}

/// 从 MessagePack 格式解码响应
///
/// # 参数
///
/// - `data`: MessagePack 编码的二进制数据
///
/// # 返回值
///
/// 解码成功返回 `Some(Response)`，失败返回 `None`
pub fn decode_response(data: &[u8]) -> Option<Response> {
    rmp_serde::from_slice(data).ok()
}
