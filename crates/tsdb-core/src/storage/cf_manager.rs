use crate::error::{Result, TsdbError};
use chrono::NaiveDate;
use rocksdb::{Options, MultiThreaded};
use std::sync::Arc;
use tracing::info;

pub const METADATA_CF: &str = "metadata";
pub const CF_PREFIX: &str = "data_";
pub const DEFAULT_HOT_DAYS: u64 = 7;
pub const DEFAULT_RETENTION_DAYS: u64 = 30;

type TsdbDB = rocksdb::DBWithThreadMode<MultiThreaded>;

#[derive(Debug, Clone)]
pub struct CfConfig {
    pub hot_days: u64,
    pub retention_days: u64,
}

impl Default for CfConfig {
    fn default() -> Self {
        Self {
            hot_days: DEFAULT_HOT_DAYS,
            retention_days: DEFAULT_RETENTION_DAYS,
        }
    }
}

pub struct CfManager {
    db: Arc<TsdbDB>,
    config: CfConfig,
    known_cfs: std::sync::Mutex<Vec<String>>,
}

impl CfManager {
    pub fn new(db: Arc<TsdbDB>, config: CfConfig) -> Self {
        let known_cfs = std::sync::Mutex::new(Vec::new());
        Self { db, config, known_cfs }
    }

    pub fn ensure_cf_for_date(&self, date: NaiveDate) -> Result<()> {
        let cf_name = format!("{}{}", CF_PREFIX, date.format("%Y%m%d"));

        if self.db.cf_handle(&cf_name).is_some() {
            return Ok(());
        }

        let is_hot = self.is_hot_date(date);
        let cf_opts = if is_hot {
            self.hot_cf_options()
        } else {
            self.cold_cf_options()
        };

        info!("creating CF {} (hot={})", cf_name, is_hot);
        self.db.create_cf(&cf_name, &cf_opts)
            .map_err(|e| TsdbError::Storage(format!("failed to create CF {}: {}", cf_name, e)))?;

        self.known_cfs.lock().unwrap().push(cf_name);
        Ok(())
    }

    pub fn get_cf_name(&self, date: NaiveDate) -> String {
        format!("{}{}", CF_PREFIX, date.format("%Y%m%d"))
    }

    pub fn cf_handle(&self, cf_name: &str) -> Result<Arc<rocksdb::BoundColumnFamily>> {
        self.db.cf_handle(cf_name)
            .ok_or_else(|| TsdbError::ColumnFamilyNotFound(cf_name.to_string()))
    }

    pub fn cleanup_expired_cfs(&self) -> Result<Vec<String>> {
        let cutoff = chrono::Local::now().date_naive()
            - chrono::Duration::days(self.config.retention_days as i64);
        let mut dropped = Vec::new();

        let cfs_to_check: Vec<String> = self.known_cfs.lock().unwrap().clone();
        for cf_name in &cfs_to_check {
            if cf_name == METADATA_CF {
                continue;
            }
            if let Some(date_str) = cf_name.strip_prefix(CF_PREFIX) {
                if let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y%m%d") {
                    if date < cutoff {
                        info!("dropping expired CF: {}", cf_name);
                        self.db.drop_cf(cf_name)
                            .map_err(|e| TsdbError::Storage(format!("failed to drop CF {}: {}", cf_name, e)))?;
                        dropped.push(cf_name.clone());
                    }
                }
            }
        }

        if !dropped.is_empty() {
            let mut known = self.known_cfs.lock().unwrap();
            known.retain(|n| !dropped.contains(n));
        }

        Ok(dropped)
    }

    fn is_hot_date(&self, date: NaiveDate) -> bool {
        let today = chrono::Local::now().date_naive();
        let diff = (today - date).num_days();
        diff >= 0 && (diff as u64) <= self.config.hot_days
    }

    fn hot_cf_options(&self) -> Options {
        let mut opts = Options::default();
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
        opts.set_level_compaction_dynamic_level_bytes(true);
        opts
    }

    fn cold_cf_options(&self) -> Options {
        let mut opts = Options::default();
        opts.set_compression_type(rocksdb::DBCompressionType::Zstd);
        opts.set_disable_auto_compactions(true);
        opts
    }
}
