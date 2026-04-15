use thiserror::Error;

#[derive(Error, Debug)]
pub enum CompressError {
    #[error("encode error: {0}")]
    Encode(String),
    #[error("decode error: {0}")]
    Decode(String),
}

pub type CompressResult<T> = std::result::Result<T, CompressError>;
