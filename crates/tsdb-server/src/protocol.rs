//! # 协议定义 — 客户端/服务端通信协议
//!
//! 使用 MessagePack 二进制序列化，支持多业务数据库隔离。
//! V2 协议增加版本号和 CRC32 校验，保证数据完整性。

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION_V1: u8 = 1;
pub const PROTOCOL_VERSION_V2: u8 = 2;
pub const CURRENT_PROTOCOL_VERSION: u8 = PROTOCOL_VERSION_V2;
pub const PROTOCOL_MAGIC: &[u8; 4] = b"TSDB";

#[derive(Debug, Serialize, Deserialize, Clone)]
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
        rmp_serde::to_vec(self).expect("Envelope serialization should not fail")
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        rmp_serde::from_slice(data).ok()
    }
}

/// 客户端请求枚举
#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    Write {
        database: String,
        measurement: String,
        tags: Vec<(String, String)>,
        fields: Vec<(String, FieldValueProto)>,
        timestamp: i64,
    },
    Query {
        database: String,
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

/// 服务端响应枚举
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
    let payload = rmp_serde::to_vec(req).expect("Request serialization should not fail");
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
    let payload = rmp_serde::to_vec(resp).expect("Response serialization should not fail");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_envelope_wrap_v2_and_validate() {
        let payload = b"hello world";
        let env = Envelope::wrap_v2(payload);
        assert_eq!(env.magic, *PROTOCOL_MAGIC);
        assert_eq!(env.version, PROTOCOL_VERSION_V2);
        assert!(env.crc32 != 0);
        assert!(env.validate());
    }

    #[test]
    fn test_envelope_wrap_v1_no_crc() {
        let payload = b"hello";
        let env = Envelope::wrap_v1(payload);
        assert_eq!(env.version, PROTOCOL_VERSION_V1);
        assert_eq!(env.crc32, 0);
    }

    #[test]
    fn test_envelope_validate_rejects_bad_magic() {
        let mut env = Envelope::wrap_v2(b"data");
        env.magic = [0xFF, 0xFF, 0xFF, 0xFF];
        assert!(!env.validate());
    }

    #[test]
    fn test_envelope_validate_rejects_tampered_payload() {
        let env = Envelope::wrap_v2(b"original data");
        let mut tampered = env.clone();
        tampered.payload = b"tampered data".to_vec();
        assert!(!tampered.validate());
    }

    #[test]
    fn test_envelope_encode_decode_roundtrip() {
        let original = Envelope::wrap_v2(b"roundtrip test data");
        let encoded = original.encode();
        let decoded = Envelope::decode(&encoded).unwrap();
        assert_eq!(decoded.magic, original.magic);
        assert_eq!(decoded.version, original.version);
        assert_eq!(decoded.payload, original.payload);
        assert!(decoded.validate());
    }

    #[test]
    fn test_envelope_decode_invalid_returns_none() {
        assert!(Envelope::decode(&[0u8; 4]).is_none());
        assert!(Envelope::decode(&[]).is_none());
    }

    #[test]
    fn test_request_write_roundtrip() {
        let req = Request::Write {
            database: "default".to_string(),
            measurement: "cpu".to_string(),
            tags: vec![("host".to_string(), "s1".to_string())],
            fields: vec![("usage".to_string(), FieldValueProto::Float(99.5))],
            timestamp: 1_000_000_000,
        };
        let encoded = encode_request(&req);
        let decoded = decode_request(&encoded).unwrap();
        match decoded {
            Request::Write {
                database,
                measurement,
                ..
            } => {
                assert_eq!(database, "default");
                assert_eq!(measurement, "cpu");
            }
            _ => panic!("expected Write"),
        }
    }

    #[test]
    fn test_request_query_roundtrip() {
        let req = Request::Query {
            database: "".to_string(),
            sql: "SELECT * FROM cpu".to_string(),
        };
        let encoded = encode_request(&req);
        let decoded = decode_request(&encoded).unwrap();
        match decoded {
            Request::Query { sql, .. } => assert_eq!(sql, "SELECT * FROM cpu"),
            _ => panic!("expected Query"),
        }
    }

    #[test]
    fn test_response_ok_roundtrip() {
        let resp = Response::Ok;
        let encoded = encode_response(&resp);
        let decoded = decode_response(&encoded).unwrap();
        assert!(matches!(decoded, Response::Ok));
    }

    #[test]
    fn test_response_error_roundtrip() {
        let resp = Response::Error("not found".to_string());
        let encoded = encode_response(&resp);
        let decoded = decode_response(&encoded).unwrap();
        match decoded {
            Response::Error(msg) => assert_eq!(msg, "not found"),
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn test_field_value_proto_conversion() {
        use tsdb_types::model::FieldValue;

        let fv = FieldValue::Float(42.5);
        let proto: FieldValueProto = fv.clone().into();
        let back: FieldValue = proto.into();
        assert_eq!(fv, back);

        let iv = FieldValue::Integer(-100);
        let proto: FieldValueProto = iv.clone().into();
        let back: FieldValue = proto.into();
        assert_eq!(iv, back);

        let sv = FieldValue::String("hello".to_string());
        let proto: FieldValueProto = sv.clone().into();
        let back: FieldValue = proto.into();
        assert_eq!(sv, back);

        let bv = FieldValue::Boolean(true);
        let proto: FieldValueProto = bv.clone().into();
        let back: FieldValue = proto.into();
        assert_eq!(bv, back);
    }
}
