//! # 压缩模块错误类型定义
//!
//! 定义 `CompressError` 枚举和 `CompressResult` 类型别名，
//! 为所有压缩/解压操作提供统一的错误处理机制。
//!

use thiserror::Error;

/// 压缩模块统一错误类型
///
/// 覆盖编码（压缩）和解码（解压）两个方向的错误场景。
#[derive(Error, Debug)]
pub enum CompressError {
    /// 编码（压缩）过程中的错误
    /// 如：位流写入溢出、数据格式不兼容等
    #[error("encode error: {0}")]
    Encode(String),

    /// 解码（解压）过程中的错误
    /// 如：数据截断、格式损坏、校验失败等
    #[error("decode error: {0}")]
    Decode(String),
}

/// 压缩模块的 Result 类型别名
///
/// 简化函数签名，避免重复书写 `std::result::Result<T, CompressError>`。
pub type CompressResult<T> = std::result::Result<T, CompressError>;
