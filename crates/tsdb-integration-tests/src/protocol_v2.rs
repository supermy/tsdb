//! 协议 V2 集成测试
//!
//! 测试覆盖：
//! 1. Envelope V2 编解码 + CRC32 校验
//! 2. Request/Response 全类型序列化/反序列化
//! 3. V1/V2 兼容性（V1 无 CRC32，V2 有 CRC32）
//! 4. 数据篡改检测

#![allow(dead_code, unused_imports)]

use tsdb_server::protocol::*;
use tsdb_types::model::FieldValue;

#[test]
fn test_envelope_v2_round_trip() {
    let payload = b"hello_tsdb_protocol_v2";
    let env = Envelope::wrap_v2(payload);
    assert_eq!(env.version, PROTOCOL_VERSION_V2);
    assert_eq!(env.magic, *PROTOCOL_MAGIC);
    assert_ne!(env.crc32, 0);
    assert!(env.validate());

    let encoded = env.encode();
    let decoded = Envelope::decode(&encoded).unwrap();
    assert_eq!(decoded.version, env.version);
    assert_eq!(decoded.crc32, env.crc32);
    assert_eq!(decoded.payload, payload);
}

#[test]
fn test_envelope_v1_no_crc() {
    let payload = b"legacy_payload";
    let env = Envelope::wrap_v1(payload);
    assert_eq!(env.version, PROTOCOL_VERSION_V1);
    assert_eq!(env.crc32, 0);
    assert!(env.validate());
}

#[test]
fn test_envelope_tamper_detection() {
    let payload = b"sensitive_data";
    let env = Envelope::wrap_v2(payload);

    let mut encoded = env.encode();

    if !encoded.is_empty() {
        let last_idx = encoded.len() - 1;
        encoded[last_idx] ^= 0xFF;
    }

    let decoded = match Envelope::decode(&encoded) {
        Some(d) => d,
        None => return, // tampered data cannot be deserialized - test passes
    };
    assert!(
        !decoded.validate(),
        "tampered envelope should fail validation"
    );
}

#[test]
fn test_request_write_encode_decode() {
    let req = Request::Write {
        database: "mydb".to_string(),
        measurement: "cpu".to_string(),
        tags: vec![
            ("host".to_string(), "server01".to_string()),
            ("region".to_string(), "us-west".to_string()),
        ],
        fields: vec![
            ("usage_user".to_string(), FieldValueProto::Float(85.5)),
            ("usage_idle".to_string(), FieldValueProto::Integer(10)),
        ],
        timestamp: 1_704_067_200_000_000,
    };

    let encoded = encode_request(&req);
    let decoded = decode_request(&encoded).unwrap();

    match decoded {
        Request::Write {
            database,
            measurement,
            tags,
            fields,
            timestamp,
        } => {
            assert_eq!(database, "mydb");
            assert_eq!(measurement, "cpu");
            assert_eq!(tags.len(), 2);
            assert_eq!(fields.len(), 2);
            assert_eq!(timestamp, 1_704_067_200_000_000);
        }
        _ => panic!("expected Write request"),
    }
}

#[test]
fn test_request_query_encode_decode() {
    let req = Request::Query {
        database: "".to_string(),
        sql: "SELECT usage FROM cpu WHERE host='server01' LIMIT 100".to_string(),
    };

    let encoded = encode_request(&req);
    let decoded = decode_request(&encoded).unwrap();

    match decoded {
        Request::Query { sql, .. } => {
            assert_eq!(sql, "SELECT usage FROM cpu WHERE host='server01' LIMIT 100");
        }
        _ => panic!("expected Query request"),
    }
}

#[test]
fn test_request_all_variants() {
    use Request::*;

    let variants: Vec<Request> = vec![
        Write {
            database: "db".to_string(),
            measurement: "m".to_string(),
            tags: vec![],
            fields: vec![],
            timestamp: 0,
        },
        Query {
            database: "".to_string(),
            sql: "SELECT 1".to_string(),
        },
        CreateDatabase {
            name: "newdb".to_string(),
        },
        ListDatabases,
        DropDatabase {
            name: "olddb".to_string(),
        },
        Ping,
    ];

    for req in &variants {
        let encoded = encode_request(req);
        let decoded = decode_request(&encoded).expect("should decode successfully");
        let re_encoded = encode_request(&decoded);
        assert_eq!(
            encoded, re_encoded,
            "round-trip should be identical for {:?}",
            req
        );
    }
}

#[test]
fn test_response_all_variants() {
    use Response::*;

    let variants: Vec<Response> = vec![
        Ok,
        Pong,
        Databases(vec!["default".to_string(), "test".to_string()]),
        Error("test error".to_string()),
        QueryResult {
            columns: vec!["time".to_string(), "value".to_string()],
            rows: vec![
                vec![FieldValueProto::Integer(100), FieldValueProto::Float(42.5)],
                vec![FieldValueProto::Integer(200), FieldValueProto::Float(43.1)],
            ],
        },
    ];

    for resp in &variants {
        let encoded = encode_response(resp);
        let decoded = decode_response(&encoded).expect("should decode successfully");
        let re_encoded = encode_response(&decoded);
        assert_eq!(
            encoded, re_encoded,
            "round-trip should be identical for {:?}",
            resp
        );
    }
}

#[test]
fn test_field_value_proto_conversions() {
    let original_values = vec![
        FieldValue::Float(std::f64::consts::PI),
        FieldValue::Integer(-42),
        FieldValue::String("hello".to_string()),
        FieldValue::Boolean(true),
    ];

    for v in original_values {
        let proto: FieldValueProto = v.clone().into();
        let round_trip: FieldValue = proto.into();
        assert_eq!(v, round_trip, "mismatch for {:?}", v);
    }
}
