//! 存储引擎集成测试
//!
//! 测试覆盖：
//! 1. StorageEngine 写入/读取基本生命周期
//! 2. 多数据库隔离

#![allow(dead_code, unused_imports)]

use tempfile::TempDir;
use tsdb_core::storage::{cf_manager::CfConfig, multi_db::MultiDbManager, StorageEngine};
use tsdb_types::model::{DataPoint, FieldValue, Tags};

#[allow(dead_code)]
fn create_test_engine(dir: &std::path::Path) -> StorageEngine {
    let cf_config = CfConfig {
        hot_days: 7,
        retention_days: 30,
    };
    StorageEngine::open(dir, cf_config).unwrap()
}

#[test]
fn test_storage_engine_lifecycle() {
    let dir = TempDir::new().unwrap();
    let engine = create_test_engine(dir.path());

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64;

    let mut dp = DataPoint::new("cpu", ts);
    dp.tags.insert("host".to_string(), "srv01".to_string());
    dp.fields
        .insert("usage".to_string(), FieldValue::Float(42.5));

    assert!(engine.write(&dp).is_ok());

    let mut tags = Tags::new();
    tags.insert("host".to_string(), "srv01".to_string());
    let result = engine.read_range("cpu", &tags, ts - 1000, ts + 1000);
    assert!(result.is_ok());
}

#[test]
fn test_multi_db_isolation() {
    let dir = TempDir::new().unwrap();
    let cf_config = CfConfig {
        hot_days: 7,
        retention_days: 30,
    };
    let mgr = MultiDbManager::new(dir.path().to_path_buf(), cf_config);

    mgr.create_database("db_a").unwrap();
    mgr.create_database("db_b").unwrap();

    assert_eq!(mgr.database_count(), 2);
    assert!(mgr.list_databases().contains(&"db_a".to_string()));
    assert!(mgr.list_databases().contains(&"db_b".to_string()));

    let db_a = mgr.get_database("db_a").unwrap();
    let db_b = mgr.get_database("db_b").unwrap();

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64;

    let mut dp_a = DataPoint::new("metric", ts);
    dp_a.fields
        .insert("val".to_string(), FieldValue::Float(1.0));
    db_a.write(&dp_a).unwrap();

    let mut dp_b = DataPoint::new("metric", ts);
    dp_b.fields
        .insert("val".to_string(), FieldValue::Float(2.0));
    db_b.write(&dp_b).unwrap();

    drop(db_a);
    drop(db_b);

    assert!(mgr.drop_database("db_b").is_ok());
    assert_eq!(mgr.database_count(), 1);
}
