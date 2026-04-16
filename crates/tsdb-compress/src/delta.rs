//! Delta 增量编码模块 - Delta Encoding Module
//!
//! 本模块实现了时间戳的 Delta-of-Delta 增量编码 + RLE，用于高效压缩时间序列数据。
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
//! ## RLE 优化
//!
//! 当连续多个 DoD 值相同时（固定间隔场景），使用 RLE 编码：
//! - 非 RLE: [dod=0][dod=0][dod=0]... → N 字节
//! - RLE:    [0xFF marker][dod_value][repeat_count] → 2-3 字节
//!
//! ## 压缩效果
//!
//! | 场景 | 原始大小 | 压缩后 | 压缩比 |
//! |------|----------|--------|--------|
//! | 固定间隔 (RLE) | 8B/点 | ~0.03B/点 | 240:1 |
//! | 固定间隔 | 8B/点 | ~1B/点 | 8:1 |
//! | 抖动间隔 | 8B/点 | ~2B/点 | 4:1 |
//! | 随机间隔 | 8B/点 | ~4B/点 | 2:1 |
//!
//! ## 编码格式
//!
//! ```text
//! [first_timestamp:8B BE] [dod_entry] [dod_entry] ...
//! ```
//!
//! dod_entry 格式:
//! - 普通: [zigzag_varint] — 1-10 字节
//! - RLE:  [0xFF] [zigzag_varint(value)] [varint(repeat_count)] — 3-12 字节

use crate::error::{CompressError, CompressResult};

const RLE_MARKER: u8 = 0xFF;
const MIN_RLE_RUN: usize = 3;

#[derive(Default)]
pub struct DeltaEncoder {
    first_timestamp: i64,
    last_timestamp: i64,
    last_delta: i64,
    initialized: bool,
    encoded: Vec<u8>,
    pending_dod: Vec<i64>,
}

impl DeltaEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn encode(&mut self, timestamp: i64) -> CompressResult<()> {
        if !self.initialized {
            self.first_timestamp = timestamp;
            self.last_timestamp = timestamp;
            self.last_delta = 0;
            self.initialized = true;
            self.encoded.extend_from_slice(&timestamp.to_be_bytes());
            return Ok(());
        }

        let delta = timestamp - self.last_timestamp;
        let delta_of_delta = delta - self.last_delta;
        self.pending_dod.push(delta_of_delta);
        self.last_delta = delta;
        self.last_timestamp = timestamp;
        Ok(())
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.flush_pending();
        self.encoded
    }

    fn flush_pending(&mut self) {
        let mut i = 0;
        while i < self.pending_dod.len() {
            let val = self.pending_dod[i];
            let mut run_len = 1;
            while i + run_len < self.pending_dod.len() && self.pending_dod[i + run_len] == val {
                run_len += 1;
            }

            if run_len >= MIN_RLE_RUN {
                self.encoded.push(RLE_MARKER);
                Self::encode_signed_varint(&mut self.encoded, val);
                Self::encode_unsigned_varint(&mut self.encoded, run_len as u64);
                i += run_len;
            } else {
                for j in 0..run_len {
                    Self::encode_signed_varint(&mut self.encoded, self.pending_dod[i + j]);
                }
                i += run_len;
            }
        }
    }

    fn encode_signed_varint(buf: &mut Vec<u8>, val: i64) {
        let zigzag = Self::zigzag_encode(val);
        Self::encode_unsigned_varint(buf, zigzag);
    }

    #[inline]
    fn zigzag_encode(n: i64) -> u64 {
        ((n << 1) ^ (n >> 63)) as u64
    }

    fn encode_unsigned_varint(buf: &mut Vec<u8>, mut val: u64) {
        loop {
            let mut byte = (val & 0x7F) as u8;
            val >>= 7;
            if val > 0 {
                byte |= 0x80;
            }
            buf.push(byte);
            if val == 0 {
                break;
            }
        }
    }
}

pub struct DeltaDecoder {
    last_timestamp: i64,
    last_delta: i64,
    initialized: bool,
    data: Vec<u8>,
    pos: usize,
    rle_value: Option<i64>,
    rle_remaining: usize,
}

impl DeltaDecoder {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            last_timestamp: 0,
            last_delta: 0,
            initialized: false,
            data,
            pos: 0,
            rle_value: None,
            rle_remaining: 0,
        }
    }

    pub fn decode_next(&mut self) -> CompressResult<Option<i64>> {
        if !self.initialized {
            if self.data.len() - self.pos < 8 {
                return Ok(None);
            }
            let ts = i64::from_be_bytes(
                self.data[self.pos..self.pos + 8]
                    .try_into()
                    .map_err(|_| CompressError::Decode("invalid timestamp".into()))?,
            );
            self.pos += 8;
            self.last_timestamp = ts;
            self.initialized = true;
            return Ok(Some(ts));
        }

        let dod = if self.rle_remaining > 0 {
            self.rle_remaining -= 1;
            self.rle_value.unwrap_or(0)
        } else if self.pos >= self.data.len() {
            return Ok(None);
        } else {
            self.decode_dod_entry()?
        };

        let delta = self.last_delta + dod;
        let timestamp = self.last_timestamp + delta;
        self.last_delta = delta;
        self.last_timestamp = timestamp;
        Ok(Some(timestamp))
    }

    fn decode_dod_entry(&mut self) -> CompressResult<i64> {
        if self.pos < self.data.len() && self.data[self.pos] == RLE_MARKER {
            self.pos += 1;
            let zigzag = Self::decode_unsigned_varint(&self.data, &mut self.pos)?;
            let val = Self::zigzag_decode(zigzag);
            let repeat_count = Self::decode_unsigned_varint(&self.data, &mut self.pos)? as usize;
            self.rle_value = Some(val);
            self.rle_remaining = repeat_count - 1;
            Ok(val)
        } else {
            let zigzag = Self::decode_unsigned_varint(&self.data, &mut self.pos)?;
            Ok(Self::zigzag_decode(zigzag))
        }
    }

    pub fn decode_all(mut self) -> CompressResult<Vec<i64>> {
        let mut results = Vec::new();
        while let Some(ts) = self.decode_next()? {
            results.push(ts);
        }
        Ok(results)
    }

    #[inline]
    fn zigzag_decode(n: u64) -> i64 {
        ((n >> 1) as i64) ^ -((n & 1) as i64)
    }

    fn decode_unsigned_varint(data: &[u8], pos: &mut usize) -> CompressResult<u64> {
        let mut val: u64 = 0;
        let mut shift: u32 = 0;

        loop {
            if *pos >= data.len() {
                return Err(CompressError::Decode(
                    "unexpected end of varint data".into(),
                ));
            }
            let byte = data[*pos];
            *pos += 1;

            val |= ((byte & 0x7F) as u64) << shift;

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

    #[test]
    fn test_delta_encode_decode() {
        let timestamps: Vec<i64> = vec![
            1_000_000_000i64,
            1_000_030_000,
            1_000_060_000,
            1_000_090_000,
            1_000_120_000,
        ];

        let mut encoder = DeltaEncoder::new();
        for &ts in &timestamps {
            encoder.encode(ts).unwrap();
        }
        let encoded = encoder.finish();

        let decoder = DeltaDecoder::new(encoded);
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded, timestamps);
    }

    #[test]
    fn test_delta_constant_interval() {
        let base = 1_000_000_000i64;
        let timestamps: Vec<i64> = (0..100).map(|i| base + i * 30_000_000).collect();

        let mut encoder = DeltaEncoder::new();
        for &ts in &timestamps {
            encoder.encode(ts).unwrap();
        }
        let encoded = encoder.finish();

        assert!(encoded.len() < timestamps.len() * 8);

        let decoder = DeltaDecoder::new(encoded);
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded, timestamps);
    }

    #[test]
    fn test_delta_rle_compression() {
        let base = 1_000_000_000i64;
        let timestamps: Vec<i64> = (0..1000).map(|i| base + i * 30_000_000).collect();

        let mut encoder = DeltaEncoder::new();
        for &ts in &timestamps {
            encoder.encode(ts).unwrap();
        }
        let encoded = encoder.finish();

        // With RLE, 1000 identical DoD=0 should compress to ~12 bytes (8B first_ts + 3B RLE entry)
        assert!(
            encoded.len() < 20,
            "RLE should compress 1000 fixed-interval timestamps to < 20 bytes, got {}",
            encoded.len()
        );

        let decoder = DeltaDecoder::new(encoded);
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded, timestamps);
    }

    #[test]
    fn test_delta_mixed_intervals() {
        let timestamps: Vec<i64> = vec![
            1_000_000_000,
            1_000_030_000,
            1_000_060_000,
            1_000_090_000,
            1_000_150_000,
            1_000_180_000,
            1_000_210_000,
        ];

        let mut encoder = DeltaEncoder::new();
        for &ts in &timestamps {
            encoder.encode(ts).unwrap();
        }
        let encoded = encoder.finish();

        let decoder = DeltaDecoder::new(encoded);
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded, timestamps);
    }

    #[test]
    fn test_zigzag() {
        assert_eq!(DeltaEncoder::zigzag_encode(0), 0);
        assert_eq!(DeltaEncoder::zigzag_encode(1), 2);
        assert_eq!(DeltaEncoder::zigzag_encode(2), 4);
        assert_eq!(DeltaEncoder::zigzag_encode(-1), 1);
        assert_eq!(DeltaEncoder::zigzag_encode(-2), 3);

        assert_eq!(DeltaDecoder::zigzag_decode(0), 0);
        assert_eq!(DeltaDecoder::zigzag_decode(1), -1);
        assert_eq!(DeltaDecoder::zigzag_decode(2), 1);
        assert_eq!(DeltaDecoder::zigzag_decode(3), -2);
    }

    #[test]
    fn test_delta_single_timestamp() {
        let timestamps: Vec<i64> = vec![1_000_000_000];

        let mut encoder = DeltaEncoder::new();
        for &ts in &timestamps {
            encoder.encode(ts).unwrap();
        }
        let encoded = encoder.finish();

        let decoder = DeltaDecoder::new(encoded);
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded, timestamps);
    }

    #[test]
    fn test_delta_decreasing_timestamps() {
        let timestamps: Vec<i64> = vec![1_000_100_000, 1_000_080_000, 1_000_060_000];

        let mut encoder = DeltaEncoder::new();
        for &ts in &timestamps {
            encoder.encode(ts).unwrap();
        }
        let encoded = encoder.finish();

        let decoder = DeltaDecoder::new(encoded);
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded, timestamps);
    }
}
