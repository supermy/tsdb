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
}

pub type Result<T> = std::result::Result<T, TsdbError>;
