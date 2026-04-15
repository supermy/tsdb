//! # 协议定义 — 客户端/服务端通信协议
//!
//! 使用 MessagePack 二进制序列化，支持多业务数据库隔离。

use serde::{Deserialize, Serialize};

/// 客户端请求枚举
#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    /// 写入数据点（支持指定目标数据库）
    Write {
        /// 目标数据库名称（空字符串表示使用 default）
        database: String,
        measurement: String,
        tags: Vec<(String, String)>,
        fields: Vec<(String, FieldValueProto)>,
        timestamp: i64,
    },

    /// SQL 查询（支持指定目标数据库）
    Query {
        /// 目标数据库名称（空字符串表示使用 default）
        database: String,
        sql: String,
    },

    /// 创建新数据库
    CreateDatabase { name: String },

    /// 列出所有数据库
    ListDatabases,

    /// 删除数据库
    DropDatabase { name: String },

    /// 健康检查
    Ping,
}

/// 服务端响应枚举
#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Ok,
    QueryResult { columns: Vec<String>, rows: Vec<Vec<FieldValueProto>> },
    Databases(Vec<String>),
    Error(String),
    Pong,
}

/// 字段值协议格式
#[derive(Debug, Serialize, Deserialize)]
pub enum FieldValueProto {
    Float(f64),
    Integer(i64),
    String(String),
    Boolean(bool),
}

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

pub fn encode_request(req: &Request) -> Vec<u8> {
    rmp_serde::to_vec(req).unwrap_or_default()
}

pub fn decode_request(data: &[u8]) -> Option<Request> {
    rmp_serde::from_slice(data).ok()
}

pub fn encode_response(resp: &Response) -> Vec<u8> {
    rmp_serde::to_vec(resp).unwrap_or_default()
}

pub fn decode_response(data: &[u8]) -> Option<Response> {
    rmp_serde::from_slice(data).ok()
}
