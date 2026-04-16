//! # TSDB 系统集成测试
//!
//! 端到端集成测试，覆盖：
//! - 存储引擎完整链路（写入→压缩→读取→查询）
//! - 聚合管道（Pipeline→Store→Timeseries）
//! - 协议 V2 编解码 + CRC32 校验
//! - TCP 客户端-服务端通信

mod aggregation_pipeline;
mod protocol_v2;
mod storage_pipeline;
mod tcp_integration;
