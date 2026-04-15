use ini::Ini;
use std::path::{Path, PathBuf};
use std::env;

#[derive(Debug, Clone)]
pub struct TsdbConfig {
    pub server: ServerConfig,
    pub storage: StorageConfig,
    pub aggregate: AggregateConfig,
    pub log: LogConfig,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub workers: usize,
}

#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub data_dir: PathBuf,
    pub hot_days: u64,
    pub retention_days: u64,
    pub block_duration_secs: u64,
    pub write_buffer_size: usize,
    pub max_open_files: i32,
}

#[derive(Debug, Clone)]
pub struct AggregateConfig {
    pub enabled: bool,
    pub worker_count: usize,
    pub time_dimensions: Vec<String>,
    pub nng_url: String,
}

#[derive(Debug, Clone)]
pub struct LogConfig {
    pub level: String,
    pub file: Option<PathBuf>,
}

impl Default for TsdbConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 7878,
                workers: 4,
            },
            storage: StorageConfig {
                data_dir: PathBuf::from("./data"),
                hot_days: 7,
                retention_days: 30,
                block_duration_secs: 30,
                write_buffer_size: 64 * 1024 * 1024,
                max_open_files: 1024,
            },
            aggregate: AggregateConfig {
                enabled: true,
                worker_count: 2,
                time_dimensions: vec![
                    "hour".to_string(),
                    "day".to_string(),
                    "week".to_string(),
                    "month".to_string(),
                ],
                nng_url: "inproc://aggregate".to_string(),
            },
            log: LogConfig {
                level: "info".to_string(),
                file: None,
            },
        }
    }
}

impl TsdbConfig {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let conf = Ini::load_from_file(path)
            .map_err(|e| ConfigError::Parse(e.to_string()))?;

        let mut config = TsdbConfig::default();

        if let Some(section) = conf.section(Some("server")) {
            config.server.host = section
                .get("host")
                .map(|s| s.to_string())
                .unwrap_or(config.server.host);
            config.server.port = section
                .get("port")
                .and_then(|s| s.parse().ok())
                .unwrap_or(config.server.port);
            config.server.workers = section
                .get("workers")
                .and_then(|s| s.parse().ok())
                .unwrap_or(config.server.workers);
        }

        if let Some(section) = conf.section(Some("storage")) {
            config.storage.data_dir = section
                .get("data_dir")
                .map(PathBuf::from)
                .unwrap_or_else(|| config.storage.data_dir);
            config.storage.hot_days = section
                .get("hot_days")
                .and_then(|s| s.parse().ok())
                .unwrap_or(config.storage.hot_days);
            config.storage.retention_days = section
                .get("retention_days")
                .and_then(|s| s.parse().ok())
                .unwrap_or(config.storage.retention_days);
            config.storage.block_duration_secs = section
                .get("block_duration_secs")
                .and_then(|s| s.parse().ok())
                .unwrap_or(config.storage.block_duration_secs);
            config.storage.write_buffer_size = section
                .get("write_buffer_size")
                .and_then(|s| s.parse().ok())
                .unwrap_or(config.storage.write_buffer_size);
            config.storage.max_open_files = section
                .get("max_open_files")
                .and_then(|s| s.parse().ok())
                .unwrap_or(config.storage.max_open_files);
        }

        if let Some(section) = conf.section(Some("aggregate")) {
            config.aggregate.enabled = section
                .get("enabled")
                .map(|s| s == "true")
                .unwrap_or(config.aggregate.enabled);
            config.aggregate.worker_count = section
                .get("worker_count")
                .and_then(|s| s.parse().ok())
                .unwrap_or(config.aggregate.worker_count);
            config.aggregate.time_dimensions = section
                .get("time_dimensions")
                .map(|s| s.split(',').map(|d| d.trim().to_string()).collect())
                .unwrap_or(config.aggregate.time_dimensions);
            config.aggregate.nng_url = section
                .get("nng_url")
                .map(|s| s.to_string())
                .unwrap_or(config.aggregate.nng_url);
        }

        if let Some(section) = conf.section(Some("log")) {
            config.log.level = section
                .get("level")
                .map(|s| s.to_string())
                .unwrap_or(config.log.level);
            config.log.file = section
                .get("file")
                .map(PathBuf::from);
        }

        config.apply_env_overrides();
        Ok(config)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(v) = env::var("TSDB_HOST") {
            self.server.host = v;
        }
        if let Ok(v) = env::var("TSDB_PORT") {
            if let Ok(port) = v.parse() {
                self.server.port = port;
            }
        }
        if let Ok(v) = env::var("TSDB_DATA_DIR") {
            self.storage.data_dir = PathBuf::from(v);
        }
        if let Ok(v) = env::var("TSDB_LOG_LEVEL") {
            self.log.level = v;
        }
        if let Ok(v) = env::var("TSDB_RETENTION_DAYS") {
            if let Ok(days) = v.parse() {
                self.storage.retention_days = days;
            }
        }
    }

    pub fn generate_default_ini(path: &Path) -> Result<(), ConfigError> {
        let mut conf = Ini::default();

        conf.with_section(Some("server"))
            .set("host", &self_default().server.host)
            .set("port", self_default().server.port.to_string())
            .set("workers", self_default().server.workers.to_string());

        conf.with_section(Some("storage"))
            .set("data_dir", self_default().storage.data_dir.to_str().unwrap_or("./data"))
            .set("hot_days", self_default().storage.hot_days.to_string())
            .set("retention_days", self_default().storage.retention_days.to_string())
            .set("block_duration_secs", self_default().storage.block_duration_secs.to_string())
            .set("write_buffer_size", self_default().storage.write_buffer_size.to_string())
            .set("max_open_files", self_default().storage.max_open_files.to_string());

        conf.with_section(Some("aggregate"))
            .set("enabled", self_default().aggregate.enabled.to_string())
            .set("worker_count", self_default().aggregate.worker_count.to_string())
            .set("time_dimensions", self_default().aggregate.time_dimensions.join(","))
            .set("nng_url", &self_default().aggregate.nng_url);

        conf.with_section(Some("log"))
            .set("level", &self_default().log.level);

        conf.write_to_file(path)
            .map_err(|e| ConfigError::Io(e.to_string()))?;

        Ok(())
    }
}

fn self_default() -> TsdbConfig {
    TsdbConfig::default()
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("parse error: {0}")]
    Parse(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("validation error: {0}")]
    Validation(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = TsdbConfig::default();
        assert_eq!(config.server.port, 7878);
        assert_eq!(config.storage.hot_days, 7);
        assert_eq!(config.storage.retention_days, 30);
        assert_eq!(config.storage.block_duration_secs, 30);
    }

    #[test]
    fn test_load_from_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.ini");

        let ini_content = r#"
[server]
host = 127.0.0.1
port = 9090

[storage]
data_dir = /tmp/tsdb_data
hot_days = 3
retention_days = 60

[log]
level = debug
"#;
        std::fs::write(&path, ini_content).unwrap();
        let config = TsdbConfig::load(&path).unwrap();
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 9090);
        assert_eq!(config.storage.data_dir, PathBuf::from("/tmp/tsdb_data"));
        assert_eq!(config.storage.hot_days, 3);
        assert_eq!(config.storage.retention_days, 60);
        assert_eq!(config.log.level, "debug");
    }

    #[test]
    fn test_generate_default_ini() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.ini");
        TsdbConfig::generate_default_ini(&path).unwrap();
        assert!(path.exists());

        let loaded = TsdbConfig::load(&path).unwrap();
        assert_eq!(loaded.server.port, 7878);
    }
}
