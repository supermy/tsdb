//! # 协议定义 — 客户端/服务端通信协议
//!
//! 使用 MessagePack 二进制序列化，支持多业务数据库隔离。
//! V2 协议增加版本号和 CRC32 校验，保证数据完整性。

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION_V1: u8 = 1;
pub const PROTOCOL_VERSION_V2: u8 = 2;
pub const CURRENT_PROTOCOL_VERSION: u8 = PROTOCOL_VERSION_V2;
pub const PROTOCOL_MAGIC: &[u8; 4] = b"TSDB";

#[derive(Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub magic: [u8; 4],
    pub version: u8,
    pub crc32: u32,
    pub payload: Vec<u8>,
}

impl Envelope {
    pub fn wrap_v2(payload: &[u8]) -> Self {
        let crc = crc32fast::hash(payload);
        Envelope {
            magic: *PROTOCOL_MAGIC,
            version: PROTOCOL_VERSION_V2,
            crc32: crc,
            payload: payload.to_vec(),
        }
    }

    pub fn wrap_v1(payload: &[u8]) -> Self {
        Envelope {
            magic: *PROTOCOL_MAGIC,
            version: PROTOCOL_VERSION_V1,
            crc32: 0,
            payload: payload.to_vec(),
        }
    }

    pub fn validate(&self) -> bool {
        if self.magic != *PROTOCOL_MAGIC {
            return false;
        }
        if self.version >= PROTOCOL_VERSION_V2 && self.crc32 != 0 {
            let computed = crc32fast::hash(&self.payload);
            return computed == self.crc32;
        }
        true
    }

    pub fn encode(&self) -> Vec<u8> {
        rmp_serde::to_vec(self).unwrap_or_default()
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        rmp_serde::from_slice(data).ok()
    }
}

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
    let payload = rmp_serde::to_vec(req).unwrap_or_default();
    let envelope = Envelope::wrap_v2(&payload);
    envelope.encode()
}

pub fn decode_request(data: &[u8]) -> Option<Request> {
    if let Ok(envelope) = rmp_serde::from_slice::<Envelope>(data) {
        if !envelope.validate() {
            return None;
        }
        return rmp_serde::from_slice(&envelope.payload).ok();
    }
    rmp_serde::from_slice(data).ok()
}

pub fn encode_response(resp: &Response) -> Vec<u8> {
    let payload = rmp_serde::to_vec(resp).unwrap_or_default();
    let envelope = Envelope::wrap_v2(&payload);
    envelope.encode()
}

pub fn decode_response(data: &[u8]) -> Option<Response> {
    if let Ok(envelope) = rmp_serde::from_slice::<Envelope>(data) {
        if !envelope.validate() {
            return None;
        }
        return rmp_serde::from_slice(&envelope.payload).ok();
    }
    rmp_serde::from_slice(data).ok()
}
