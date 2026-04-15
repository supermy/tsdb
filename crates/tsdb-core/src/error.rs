use thiserror::Error;

#[derive(Error, Debug)]
pub enum TsdbError {
    #[error("storage error: {0}")]
    Storage(String),

    #[error("invalid data point: {0}")]
    InvalidDataPoint(String),

    #[error("column family not found: {0}")]
    ColumnFamilyNotFound(String),

    #[error("compression error: {0}")]
    Compression(String),

    #[error("decompression error: {0}")]
    Decompression(String),

    #[error("index error: {0}")]
    Index(String),

    #[error("query error: {0}")]
    Query(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("rocksdb error: {0}")]
    RocksDb(#[from] rocksdb::Error),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("plugin error: {0}")]
    Plugin(String),

    #[error("nng error: {0}")]
    Nng(String),

    #[error("dashboard error: {0}")]
    Dashboard(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl From<serde_json::Error> for TsdbError {
    fn from(e: serde_json::Error) -> Self {
        TsdbError::Serialization(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, TsdbError>;
