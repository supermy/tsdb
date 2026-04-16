//! # 多业务数据库管理器 — 支持业务级数据隔离
//!
//! ## 设计目标
//!
//! 不同业务（股票行情、IOT、金融、订单等）的数据存储在独立的 RocksDB 实例中，
//! 实现物理级别的数据隔离，互不干扰。
//!
//! ```text
//! MultiDbManager
//! ├── data_dir/
//! │   ├── default/     ← 默认数据库 (RocksDB 实例)
//! │   ├── stocks/      ← 股票行情数据库
//! │   ├── iot/         ← 物联网数据库
//! │   └── finance/     ← 金融数据库
//! │
//! └── databases: HashMap<String, Arc<StorageEngine>>
//!     "default" → StorageEngine (data_dir/default/)
//!     "stocks"  → StorageEngine (data_dir/stocks/)
//!     "iot"     → StorageEngine (data_dir/iot/)
//! ```
//!
//! ## 线程安全
//!
//! 使用 `RwLock<HashMap>` 保护内部映射表，支持多线程并发读取和写入。

use crate::error::{Result, TsdbError};
use crate::storage::cf_manager::CfConfig;
use crate::storage::StorageEngine;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tracing::{info, warn};

/// 多业务数据库管理器 — 统一管理多个独立的 StorageEngine 实例
pub struct MultiDbManager {
    /// 数据根目录（所有 DB 实例的父目录）
    data_dir: PathBuf,
    /// 列族配置（热数据天数、保留天数）
    cf_config: CfConfig,
    /// 数据库名称 → StorageEngine 实例的线程安全映射表
    databases: RwLock<HashMap<String, Arc<StorageEngine>>>,
}

impl MultiDbManager {
    /// 创建新的多数据库管理器
    ///
    /// # 参数
    /// - `data_dir`: 数据根目录路径
    /// - `cf_config`: 列族配置（所有 DB 实例共享同一配置）
    pub fn new(data_dir: PathBuf, cf_config: CfConfig) -> Self {
        Self {
            data_dir,
            cf_config,
            databases: RwLock::new(HashMap::new()),
        }
    }

    /// 创建新的数据库实例
    ///
    /// 在 `data_dir/<name>/` 下创建独立的 RocksDB 实例。
    /// 如果同名数据库已存在则返回错误。
    ///
    /// # 参数
    /// - `name`: 数据库名称（如 `"stocks"`, `"iot"`）
    pub fn create_database(&self, name: &str) -> Result<Arc<StorageEngine>> {
        {
            let dbs = self.databases.read().unwrap();
            if dbs.contains_key(name) {
                return Err(TsdbError::Storage(format!(
                    "database '{}' already exists",
                    name
                )));
            }
        }

        let db_path = self.data_dir.join(name);
        std::fs::create_dir_all(&db_path)?;

        let engine = StorageEngine::open(&db_path, self.cf_config.clone())?;
        let engine = Arc::new(engine);

        self.databases
            .write()
            .unwrap()
            .insert(name.to_string(), Arc::clone(&engine));

        info!("database '{}' created at {:?}", name, db_path);
        Ok(engine)
    }

    /// 获取指定名称的数据库实例
    ///
    /// # 参数
    /// - `name`: 数据库名称
    ///
    /// # 返回
    /// - `Ok(Arc<StorageEngine>)`: 找到对应实例
    /// - `Err(TsdbError::NotFound)`: 数据库不存在
    pub fn get_database(&self, name: &str) -> Result<Arc<StorageEngine>> {
        self.databases
            .read()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| TsdbError::NotFound(format!("database '{}'", name)))
    }

    /// 删除指定名称的数据库实例
    ///
    /// 先从内存映射表中移除，再删除磁盘上的数据目录。
    /// 注意：此操作不可逆，删除后数据无法恢复。
    ///
    /// # 参数
    /// - `name`: 待删除的数据库名称
    pub fn drop_database(&self, name: &str) -> Result<()> {
        if name == "default" {
            return Err(TsdbError::Storage(
                "cannot drop the default database".into(),
            ));
        }

        let removed = self.databases.write().unwrap().remove(name);
        if removed.is_none() {
            return Err(TsdbError::NotFound(format!("database '{}'", name)));
        }

        let db_path = self.data_dir.join(name);
        if db_path.exists() {
            std::fs::remove_dir_all(&db_path)?;
        }

        info!("database '{}' dropped", name);
        Ok(())
    }

    /// 列出所有已注册的数据库名称
    pub fn list_databases(&self) -> Vec<String> {
        self.databases.read().unwrap().keys().cloned().collect()
    }

    /// 返回已注册的数据库数量
    pub fn database_count(&self) -> usize {
        self.databases.read().unwrap().len()
    }

    /// 确保默认数据库已初始化（服务启动时调用）
    pub fn ensure_default(&self) -> Result<Arc<StorageEngine>> {
        {
            let dbs = self.databases.read().unwrap();
            if let Some(engine) = dbs.get("default") {
                return Ok(Arc::clone(engine));
            }
        }
        self.create_database("default")
    }

    /// 清理所有数据库的过期列族
    pub fn cleanup_all(&self) -> Result<Vec<(String, Vec<String>)>> {
        let mut results = Vec::new();
        let dbs = self.databases.read().unwrap();
        for (name, engine) in dbs.iter() {
            match engine.cleanup() {
                Ok(dropped) if !dropped.is_empty() => {
                    results.push((name.clone(), dropped));
                }
                Err(e) => {
                    warn!("cleanup failed for database '{}': {}", name, e);
                }
                _ => {}
            }
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_list_databases() {
        let dir = tempfile::TempDir::new().unwrap();
        let cf_config = CfConfig::default();
        let manager = MultiDbManager::new(dir.path().to_path_buf(), cf_config);

        manager.create_database("stocks").unwrap();
        manager.create_database("iot").unwrap();

        let dbs = manager.list_databases();
        assert!(dbs.contains(&"stocks".to_string()));
        assert!(dbs.contains(&"iot".to_string()));
        assert_eq!(manager.database_count(), 2);
    }

    #[test]
    fn test_duplicate_database() {
        let dir = tempfile::TempDir::new().unwrap();
        let cf_config = CfConfig::default();
        let manager = MultiDbManager::new(dir.path().to_path_buf(), cf_config);

        manager.create_database("test").unwrap();
        let result = manager.create_database("test");
        assert!(result.is_err());
    }

    #[test]
    fn test_drop_database() {
        let dir = tempfile::TempDir::new().unwrap();
        let cf_config = CfConfig::default();
        let manager = MultiDbManager::new(dir.path().to_path_buf(), cf_config);

        manager.create_database("temp").unwrap();
        manager.drop_database("temp").unwrap();
        assert!(!manager.list_databases().contains(&"temp".to_string()));
    }

    #[test]
    fn test_cannot_drop_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let cf_config = CfConfig::default();
        let manager = MultiDbManager::new(dir.path().to_path_buf(), cf_config);

        manager.ensure_default().unwrap();
        let result = manager.drop_database("default");
        assert!(result.is_err());
    }

    #[test]
    fn test_ensure_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let cf_config = CfConfig::default();
        let manager = MultiDbManager::new(dir.path().to_path_buf(), cf_config);

        let _engine = manager.ensure_default().unwrap();
        assert!(manager.list_databases().contains(&"default".to_string()));

        let _engine2 = manager.ensure_default().unwrap();
        assert_eq!(manager.database_count(), 1);
    }

    #[test]
    fn test_get_nonexistent_database() {
        let dir = tempfile::TempDir::new().unwrap();
        let cf_config = CfConfig::default();
        let manager = MultiDbManager::new(dir.path().to_path_buf(), cf_config);

        let result = manager.get_database("nonexistent");
        assert!(result.is_err());
    }
}
