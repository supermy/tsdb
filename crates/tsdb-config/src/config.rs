//! # 配置管理 — TSDB 全局配置加载与默认值
//!
//! ## 配置文件格式（INI）
//!
//! ```ini
//! [server]
//! host = 0.0.0.0
//! port = 7878
//! workers = 4
//!
//! [storage]
//! data_dir = ./data
//! hot_days = 7
//! retention_days = 30
//! block_duration_secs = 30
//!
//! [aggregate]
//! enabled = true
//! worker_count = 2
//! time_dimensions = hour,day,week,month
//!
//! [log]
//! level = info
//! ```
//!

use ini::Ini;
use std::env;
use std::path::{Path, PathBuf};

/// TSDB 全局配置 — 包含所有子系统的运行参数
#[derive(Debug, Clone)]
pub struct TsdbConfig {
    /// TCP/HTTP 服务器配置
    pub server: ServerConfig,
    /// RocksDB 存储引擎配置
    pub storage: StorageConfig,
    /// 聚合引擎配置
    pub aggregate: AggregateConfig,
    /// 日志系统配置
    pub log: LogConfig,
}

/// 服务器网络配置
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// 监听地址（如 `"0.0.0.0"` 表示监听所有网卡）
    pub host: String,
    /// TCP 端口号（HTTP 端口自动设为 port + 1）
    pub port: u16,
    /// 工作线程数（预留参数，当前为单线程模型）
    pub workers: usize,
}

/// 存储引擎配置
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// RocksDB 数据根目录路径
    pub data_dir: PathBuf,
    /// 热数据天数（0~N 天的 CF 使用 HOT 模式，LZ4 压缩 + 动态层级压缩）
    pub hot_days: u64,
    /// 数据保留天数（超过此期限的 CF 将被自动清理删除）
    pub retention_days: u64,
    /// 时间块持续时间（秒），默认 30 秒
    pub block_duration_secs: u64,
    /// RocksDB 写缓冲区大小（字节），默认 64MB
    pub write_buffer_size: usize,
    /// RocksDB 最大打开文件数限制
    pub max_open_files: i32,
}

/// 聚合引擎配置
#[derive(Debug, Clone)]
pub struct AggregateConfig {
    /// 是否启用后台聚合工作器
    pub enabled: bool,
    /// 后台聚合线程数
    pub worker_count: usize,
    /// 需要计算的时间维度列表（如 `["hour", "day", "week", "month"]`）
    pub time_dimensions: Vec<String>,
    /// NNG 内部通信地址（用于 Worker 与 Server 之间的数据传递）
    pub nng_url: String,
}

/// 日志系统配置
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// 日志级别：`trace` / `debug` / `info` / `warn` / `error`
    pub level: String,
    /// 日志文件路径（None 表示仅输出到 stderr）
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
    /// 从 INI 配置文件加载完整配置
    ///
    /// 加载流程：
    /// 1. 创建默认配置作为基础值
    /// 2. 解析 INI 文件各 section，逐字段覆盖默认值
    /// 3. 应用环境变量覆盖（优先级最高）
    ///
    /// ## 环境变量覆盖规则
    ///
    /// | 环境变量 | 对应配置项 |
    /// |---------|-----------|
    /// | `TSDB_HOST` | server.host |
    /// | `TSDB_PORT` | server.port |
    /// | `TSDB_DATA_DIR` | storage.data_dir |
    /// | `TSDB_LOG_LEVEL` | log.level |
    /// | `TSDB_RETENTION_DAYS` | storage.retention_days |
    ///
    /// # 参数
    /// - `path`: INI 配置文件的绝对或相对路径
    ///
    /// # 返回
    /// - `Ok(TsdbConfig)`: 合并后的完整配置
    /// - `Err(ConfigError)`: 文件不存在或格式错误
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let conf = Ini::load_from_file(path).map_err(|e| ConfigError::Parse(e.to_string()))?;

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
            config.log.file = section.get("file").map(PathBuf::from);
        }

        config.apply_env_overrides();
        Ok(config)
    }

    /// 从配置文件加载，失败时返回默认配置（容错模式）
    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    /// 应用环境变量覆盖到当前配置（优先级最高）
    ///
    /// 仅覆盖已设置的环境变量，未设置的保持 INI 或默认值不变。
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

    /// 生成包含所有默认值的示例 INI 配置文件
    ///
    /// 用于首次部署时快速生成可编辑的配置模板。
    ///
    /// # 参数
    /// - `path`: 输出文件路径
    pub fn generate_default_ini(path: &Path) -> Result<(), ConfigError> {
        let mut conf = Ini::default();

        conf.with_section(Some("server"))
            .set("host", &self_default().server.host)
            .set("port", self_default().server.port.to_string())
            .set("workers", self_default().server.workers.to_string());

        conf.with_section(Some("storage"))
            .set(
                "data_dir",
                self_default().storage.data_dir.to_str().unwrap_or("./data"),
            )
            .set("hot_days", self_default().storage.hot_days.to_string())
            .set(
                "retention_days",
                self_default().storage.retention_days.to_string(),
            )
            .set(
                "block_duration_secs",
                self_default().storage.block_duration_secs.to_string(),
            )
            .set(
                "write_buffer_size",
                self_default().storage.write_buffer_size.to_string(),
            )
            .set(
                "max_open_files",
                self_default().storage.max_open_files.to_string(),
            );

        conf.with_section(Some("aggregate"))
            .set("enabled", self_default().aggregate.enabled.to_string())
            .set(
                "worker_count",
                self_default().aggregate.worker_count.to_string(),
            )
            .set(
                "time_dimensions",
                self_default().aggregate.time_dimensions.join(","),
            )
            .set("nng_url", &self_default().aggregate.nng_url);

        conf.with_section(Some("log"))
            .set("level", &self_default().log.level);

        conf.write_to_file(path)
            .map_err(|e| ConfigError::Io(e.to_string()))?;

        Ok(())
    }
}

/// 返回默认配置实例（辅助函数，用于 generate_default_ini 中避免借用冲突）
fn self_default() -> TsdbConfig {
    TsdbConfig::default()
}

/// 配置加载/解析错误类型
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
