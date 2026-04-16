//! Delta 增量编码模块 - Delta Encoding Module
//!
//! 本模块实现了时间戳的 Delta-of-Delta 增量编码，用于高效压缩时间序列数据。
//!
//! ## 编码原理
//!
//! 时序数据的时间戳通常具有以下特点：
//! 1. 单调递增（或基本单调）
//! 2. 相邻时间戳间隔相对稳定（如每 10 秒采集一次）
//! 3. Delta-of-Delta（增量的增量）通常接近 0
//!
//! ## 编码流程
//!
//! ```text
//! 原始时间戳:     t0,    t1,      t2,      t3,      ...
//! Delta:          -,  t1-t0,  t2-t1,  t3-t2,  ...
//! Delta-of-Delta: -,      -,  d2-d1,  d3-d2,  ...
//! ```
//!
//! ## 压缩效果
//!
//! | 场景 | 原始大小 | 压缩后 | 压缩比 |
//! |------|----------|--------|--------|
//! | 固定间隔 | 8B/点 | ~1B/点 | 8:1 |
//! | 抖动间隔 | 8B/点 | ~2B/点 | 4:1 |
//! | 随机间隔 | 8B/点 | ~4B/点 | 2:1 |
//!
//! ## 编码格式
//!
//! ```text
//! [first_timestamp:8B BE] [dod_1:varint] [dod_2:varint] ...
//! ```
//!
//! 其中 dod 使用 Zigzag + Varint 编码，支持负数和可变长度。

use crate::error::{CompressError, CompressResult};

/// Delta 增量编码器
///
/// 使用 Delta-of-Delta + Zigzag + Varint 算法压缩时间戳序列。
///
/// # 使用示例
///
/// ```rust,ignore
/// let mut encoder = DeltaEncoder::new();
/// encoder.encode(1000)?;
/// encoder.encode(1030)?;
/// encoder.encode(1060)?;
/// let compressed = encoder.finish();
/// ```
#[derive(Default)]
pub struct DeltaEncoder {
    first_timestamp: i64,
    last_timestamp: i64,
    last_delta: i64,
    initialized: bool,
    encoded: Vec<u8>,
}

impl DeltaEncoder {
    pub fn new() -> Self {
        Self::default()
    }
}

impl DeltaEncoder {
    /// 编码单个时间戳
    ///
    /// - `timestamp`: 时间戳（微秒）
    ///
    /// # 返回值
    ///
    /// 成功返回 `Ok(())`，失败返回错误
    ///
    /// # 算法
    ///
    /// 1. 第一个时间戳：直接存储 8 字节大端序
    /// 2. 后续时间戳：
    ///    - 计算 Delta = timestamp - last_timestamp
    ///    - 计算 Delta-of-Delta = Delta - last_delta
    ///    - 使用 Zigzag + Varint 编码
    pub fn encode(&mut self, timestamp: i64) -> CompressResult<()> {
        if !self.initialized {
            // 第一个时间戳：直接存储
            self.first_timestamp = timestamp;
            self.last_timestamp = timestamp;
            self.last_delta = 0;
            self.initialized = true;
            self.encoded.extend_from_slice(&timestamp.to_be_bytes());
            return Ok(());
        }

        // 计算 Delta 和 Delta-of-Delta
        let delta = timestamp - self.last_timestamp;
        let delta_of_delta = delta - self.last_delta;

        // 编码 Delta-of-Delta
        self.encode_varint(delta_of_delta);

        // 更新状态
        self.last_delta = delta;
        self.last_timestamp = timestamp;
        Ok(())
    }

    /// 完成编码并返回压缩数据
    pub fn finish(self) -> Vec<u8> {
        self.encoded
    }

    /// 使用 Zigzag + Varint 编码有符号整数
    ///
    /// # 参数
    ///
    /// - `val`: 有符号整数值
    fn encode_varint(&mut self, val: i64) {
        // Zigzag 编码：将负数映射到正数
        let zigzag = Self::zigzag_encode(val);
        // Varint 编码：可变长度
        Self::encode_unsigned_varint(&mut self.encoded, zigzag);
    }

    /// Zigzag 编码
    ///
    /// 将有符号整数映射为无符号整数：
    /// - 0 → 0
    /// - -1 → 1
    /// - 1 → 2
    /// - -2 → 3
    /// - 2 → 4
    ///
    /// 这样小负数也能用少量字节编码。
    #[inline]
    fn zigzag_encode(n: i64) -> u64 {
        ((n << 1) ^ (n >> 63)) as u64
    }

    /// Varint 编码（无符号）
    ///
    /// 每个字节使用 7 位存储数据，最高位表示是否继续：
    /// - 最高位 = 1：后面还有字节
    /// - 最高位 = 0：这是最后一个字节
    fn encode_unsigned_varint(buf: &mut Vec<u8>, mut val: u64) {
        loop {
            let mut byte = (val & 0x7F) as u8;
            val >>= 7;
            if val > 0 {
                byte |= 0x80;  // 设置继续标志
            }
            buf.push(byte);
            if val == 0 {
                break;
            }
        }
    }
}

/// Delta 增量解码器
///
/// 从压缩数据中恢复原始时间戳序列。
///
/// # 使用示例
///
/// ```rust,ignore
/// let decoder = DeltaDecoder::new(compressed_data);
/// let timestamps = decoder.decode_all()?;
/// ```
pub struct DeltaDecoder {
    /// 上一个时间戳
    last_timestamp: i64,
    /// 上一个 Delta 值
    last_delta: i64,
    /// 是否已初始化
    initialized: bool,
    /// 压缩数据
    data: Vec<u8>,
    /// 当前读取位置
    pos: usize,
}

impl DeltaDecoder {
    /// 从压缩数据创建解码器
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            last_timestamp: 0,
            last_delta: 0,
            initialized: false,
            data,
            pos: 0,
        }
    }

    /// 解码下一个时间戳
    ///
    /// # 返回值
    ///
    /// - `Ok(Some(timestamp))`: 成功解码一个时间戳
    /// - `Ok(None)`: 已到达数据末尾
    /// - `Err(e)`: 解码错误
    pub fn decode_next(&mut self) -> CompressResult<Option<i64>> {
        if !self.initialized {
            // 读取第一个时间戳（8 字节大端序）
            if self.data.len() - self.pos < 8 {
                return Ok(None);
            }
            let ts = i64::from_be_bytes(
                self.data[self.pos..self.pos + 8].try_into().map_err(|_| CompressError::Decode("invalid timestamp".into()))?
            );
            self.pos += 8;
            self.last_timestamp = ts;
            self.initialized = true;
            return Ok(Some(ts));
        }

        // 检查是否到达末尾
        if self.pos >= self.data.len() {
            return Ok(None);
        }

        // 解码 Delta-of-Delta
        let zigzag = Self::decode_unsigned_varint(&self.data, &mut self.pos)?;
        let dod = Self::zigzag_decode(zigzag);

        // 恢复 Delta 和时间戳
        let delta = self.last_delta + dod;
        let timestamp = self.last_timestamp + delta;

        // 更新状态
        self.last_delta = delta;
        self.last_timestamp = timestamp;

        Ok(Some(timestamp))
    }

    /// 解码所有时间戳
    pub fn decode_all(mut self) -> CompressResult<Vec<i64>> {
        let mut results = Vec::new();
        while let Some(ts) = self.decode_next()? {
            results.push(ts);
        }
        Ok(results)
    }

    /// Zigzag 解码
    #[inline]
    fn zigzag_decode(n: u64) -> i64 {
        ((n >> 1) as i64) ^ -((n & 1) as i64)
    }

    /// Varint 解码（无符号）
    fn decode_unsigned_varint(data: &[u8], pos: &mut usize) -> CompressResult<u64> {
        let mut val: u64 = 0;
        let mut shift: u32 = 0;

        loop {
            if *pos >= data.len() {
                return Err(CompressError::Decode("unexpected end of varint data".into()));
            }
            let byte = data[*pos];
            *pos += 1;

            // 提取低 7 位数据
            val |= ((byte & 0x7F) as u64) << shift;

            // 检查是否继续
            if byte & 0x80 == 0 {
                break;
            }

            shift += 7;
            if shift >= 64 {
                return Err(CompressError::Decode("varint too large".into()));
            }
        }

        Ok(val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试基本编解码
    #[test]
    fn test_delta_encode_decode() {
        let timestamps: Vec<i64> = vec![
            1_000_000_000i64,
            1_000_030_000,
            1_000_060_000,
            1_000_090_000,
            1_000_120_000,
        ];

        // 编码
        let mut encoder = DeltaEncoder::new();
        for &ts in &timestamps {
            encoder.encode(ts).unwrap();
        }
        let encoded = encoder.finish();

        // 解码
        let decoder = DeltaDecoder::new(encoded);
        let decoded = decoder.decode_all().unwrap();

        assert_eq!(decoded, timestamps);
    }

    /// 测试固定间隔场景（最佳压缩）
    #[test]
    fn test_delta_constant_interval() {
        let base = 1_000_000_000i64;
        let timestamps: Vec<i64> = (0..100).map(|i| base + i * 30_000_000).collect();

        // 编码
        let mut encoder = DeltaEncoder::new();
        for &ts in &timestamps {
            encoder.encode(ts).unwrap();
        }
        let encoded = encoder.finish();

        // 验证压缩效果：固定间隔时 Delta-of-Delta = 0，只需 1 字节
        assert!(encoded.len() < timestamps.len() * 8);

        // 解码验证
        let decoder = DeltaDecoder::new(encoded);
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded, timestamps);
    }

    /// 测试 Zigzag 编解码
    #[test]
    fn test_zigzag() {
        // 正数：乘 2
        assert_eq!(DeltaEncoder::zigzag_encode(0), 0);
        assert_eq!(DeltaEncoder::zigzag_encode(1), 2);
        assert_eq!(DeltaEncoder::zigzag_encode(2), 4);

        // 负数：乘 2 加 1
        assert_eq!(DeltaEncoder::zigzag_encode(-1), 1);
        assert_eq!(DeltaEncoder::zigzag_encode(-2), 3);

        // 解码验证
        assert_eq!(DeltaDecoder::zigzag_decode(0), 0);
        assert_eq!(DeltaDecoder::zigzag_decode(1), -1);
        assert_eq!(DeltaDecoder::zigzag_decode(2), 1);
        assert_eq!(DeltaDecoder::zigzag_decode(3), -2);
    }
}
