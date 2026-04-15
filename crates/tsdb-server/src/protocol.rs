use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    Write {
        measurement: String,
        tags: Vec<(String, String)>,
        fields: Vec<(String, FieldValueProto)>,
        timestamp: i64,
    },
    Query {
        sql: String,
    },
    CreateDatabase {
        name: String,
    },
    ListDatabases,
    DropDatabase {
        name: String,
    },
    Ping,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Ok,
    QueryResult {
        columns: Vec<String>,
        rows: Vec<Vec<FieldValueProto>>,
    },
    Databases(Vec<String>),
    Error(String),
    Pong,
}

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
