//! Simple8b 整数编码模块 - Simple8b Integer Encoding Module
//!
//! Simple8b 是一种高效的 64-bit 字对齐整数压缩算法，由 Vo Ngoc Anh 和
//! Alistair Moffat 在 2010 年提出。InfluxDB 使用此算法压缩时间戳
//! Delta-of-Delta 值和整数字段。
//!
//! ## 编码原理
//!
//! 每个 64-bit word 的结构：
//! ```text
//! ┌──────────┬──────────────────────────────────────────────┐
//! │ Selector │                    Values                     │
//! │  (4 bit) │                  (60 bit)                     │
//! └──────────┴──────────────────────────────────────────────┘
//! ```
//!
//! Selector 决定了 60 位中存储多少个值以及每个值的位宽：
//!
//! | Sel | 值数量 | 位宽/值 | 总位数 | 适用场景 |
//! |-----|--------|---------|--------|---------|
//! | 0   | 240    | 0 bit   | 0      | 全零值 (RLE) |
//! | 1   | 120    | 0 bit   | 0      | 同值重复 (RLE) |
//! | 2   | 60     | 1 bit   | 60     | 0/1 值 |
//! | 3   | 30     | 2 bit   | 60     | 小整数 |
//! | ... | ...    | ...     | ...    | ... |
//! | 14  | 2      | 20 bit  | 40     | 中等整数 |
//! | 15  | 1      | 30 bit  | 30     | 较大整数 |
//!
//! ## 压缩效果
//!
//! | 场景 | 原始大小 | 压缩后 | 压缩比 |
//! |------|----------|--------|--------|
//! | 全零 (DoD=0) | 8B/值 | 0.03B/值 | 240:1 |
//! | 1-bit 值 | 8B/值 | 1.07B/值 | 7.5:1 |
//! | 4-bit 值 | 8B/值 | 4B/值 | 2:1 |

use crate::error::{CompressError, CompressResult};

const VALUE_BITS: usize = 60;

#[rustfmt::skip]
const SELECTORS: [(usize, usize); 16] = [
    (240, 0),   // selector 0:  240 values, 0 bits each (all zeros, RLE)
    (120, 0),   // selector 1:  120 values, 0 bits each (same value, RLE)
    (60,  1),   // selector 2:   60 values, 1 bit each
    (30,  2),   // selector 3:   30 values, 2 bits each
    (20,  3),   // selector 4:   20 values, 3 bits each
    (15,  4),   // selector 5:   15 values, 4 bits each
    (12,  5),   // selector 6:   12 values, 5 bits each
    (10,  6),   // selector 7:   10 values, 6 bits each
    (8,   7),   // selector 8:    8 values, 7 bits each
    (7,   8),   // selector 9:    7 values, 8 bits each
    (6,   9),   // selector 10:   6 values, 9 bits each
    (5,  10),   // selector 11:   5 values, 10 bits each
    (4,  12),   // selector 12:   4 values, 12 bits each
    (3,  15),   // selector 13:   3 values, 15 bits each
    (2,  20),   // selector 14:   2 values, 20 bits each
    (1,  30),   // selector 15:   1 value,  30 bits each
];

#[derive(Default)]
pub struct Simple8bEncoder {
    encoded: Vec<u64>,
    total_values: usize,
}

impl Simple8bEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn encode(&mut self, values: &[u64]) -> CompressResult<()> {
        self.total_values = values.len();
        let mut i = 0;
        while i < values.len() {
            let (word, consumed) = self.encode_word(&values[i..])?;
            self.encoded.push(word);
            i += consumed;
        }
        Ok(())
    }

    fn encode_word(&mut self, values: &[u64]) -> CompressResult<(u64, usize)> {
        if values.is_empty() {
            return Err(CompressError::Encode("empty values".into()));
        }

        // Selector 0: all zeros RLE (240 zeros in one word)
        if values.len() >= 240 && values[..240].iter().all(|&v| v == 0) {
            let word = 0u64; // selector 0, no value bits
            return Ok((word, 240));
        }
        if values.iter().all(|&v| v == 0) {
            let n = values.len().min(240);
            let word = 0u64;
            return Ok((word, n));
        }

        // Selector 1: same value RLE (120 same values in one word)
        if values.len() >= 120 {
            let first = values[0];
            if values[..120].iter().all(|&v| v == first) && first < (1u64 << 60) {
                let word = (1u64 << VALUE_BITS) | first;
                return Ok((word, 120));
            }
        }
        if values.len() >= 2 {
            let first = values[0];
            let run_len = values
                .iter()
                .take(120)
                .position(|&v| v != first)
                .unwrap_or(values.len().min(120));
            if run_len >= 2 && first < (1u64 << 60) {
                let word = (1u64 << VALUE_BITS) | first;
                return Ok((word, run_len));
            }
        }

        // Selectors 2-15: bit-packing
        for (sel_idx, &(count, bits)) in SELECTORS.iter().enumerate().skip(2) {
            let n = values.len().min(count);
            let mut can_encode = true;
            for &v in &values[..n] {
                if v >= (1u64 << bits) {
                    can_encode = false;
                    break;
                }
            }

            if can_encode {
                let mut word = (sel_idx as u64) << VALUE_BITS;
                for (j, &v) in values[..n].iter().enumerate() {
                    word |= v << (j * bits);
                }
                return Ok((word, n));
            }
        }

        Err(CompressError::Encode(
            "value too large for simple8b encoding (max 30 bits)".into(),
        ))
    }

    pub fn finish(self) -> Vec<u8> {
        let word_count = self.encoded.len() as u64;
        let total_values = self.total_values as u64;
        let mut buf = Vec::with_capacity(16 + self.encoded.len() * 8);
        buf.extend_from_slice(&total_values.to_be_bytes());
        buf.extend_from_slice(&word_count.to_be_bytes());
        for &word in &self.encoded {
            buf.extend_from_slice(&word.to_be_bytes());
        }
        buf
    }
}

pub struct Simple8bDecoder {
    words: Vec<u64>,
    total_values: usize,
    values_decoded: usize,
    pos: usize,
    values_in_word: Vec<u64>,
    word_pos: usize,
}

impl Simple8bDecoder {
    pub fn new(data: Vec<u8>) -> CompressResult<Self> {
        if data.len() < 16 {
            return Err(CompressError::Decode(
                "insufficient data for simple8b header".into(),
            ));
        }

        let total_values = u64::from_be_bytes(
            data[..8]
                .try_into()
                .map_err(|_| CompressError::Decode("invalid total count".into()))?,
        ) as usize;

        let word_count = u64::from_be_bytes(
            data[8..16]
                .try_into()
                .map_err(|_| CompressError::Decode("invalid word count".into()))?,
        ) as usize;

        if data.len() < 16 + word_count * 8 {
            return Err(CompressError::Decode(
                "insufficient data for simple8b words".into(),
            ));
        }

        let mut words = Vec::with_capacity(word_count);
        for i in 0..word_count {
            let offset = 16 + i * 8;
            let word = u64::from_be_bytes(
                data[offset..offset + 8]
                    .try_into()
                    .map_err(|_| CompressError::Decode("invalid word".into()))?,
            );
            words.push(word);
        }

        Ok(Self {
            words,
            total_values,
            values_decoded: 0,
            pos: 0,
            values_in_word: Vec::new(),
            word_pos: 0,
        })
    }

    fn decode_word(&mut self, word: u64) -> Vec<u64> {
        let sel_idx = (word >> VALUE_BITS) as usize;
        if sel_idx >= SELECTORS.len() {
            return Vec::new();
        }

        let (count, bits) = SELECTORS[sel_idx];

        if bits == 0 && sel_idx == 0 {
            let remaining = self.total_values - self.values_decoded;
            let n = remaining.min(count);
            return vec![0u64; n];
        }

        if bits == 0 && sel_idx == 1 {
            let value = word & ((1u64 << 60) - 1);
            let remaining = self.total_values - self.values_decoded;
            let n = remaining.min(count);
            return vec![value; n];
        }

        let remaining = self.total_values - self.values_decoded;
        let n = remaining.min(count);
        let mask = (1u64 << bits) - 1;
        let mut values = Vec::with_capacity(n);
        for i in 0..n {
            let v = (word >> (i * bits)) & mask;
            values.push(v);
        }
        values
    }

    pub fn decode_next(&mut self) -> CompressResult<Option<u64>> {
        if self.values_decoded >= self.total_values {
            return Ok(None);
        }

        if self.word_pos < self.values_in_word.len() {
            let v = self.values_in_word[self.word_pos];
            self.word_pos += 1;
            self.values_decoded += 1;
            return Ok(Some(v));
        }

        if self.pos >= self.words.len() {
            return Ok(None);
        }

        self.values_in_word = self.decode_word(self.words[self.pos]);
        self.pos += 1;
        self.word_pos = 0;

        if self.values_in_word.is_empty() {
            return Ok(None);
        }

        let v = self.values_in_word[self.word_pos];
        self.word_pos += 1;
        self.values_decoded += 1;
        Ok(Some(v))
    }

    pub fn decode_all(mut self) -> CompressResult<Vec<u64>> {
        let mut results = Vec::with_capacity(self.total_values);
        while let Some(v) = self.decode_next()? {
            results.push(v);
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple8b_zeros() {
        let values: Vec<u64> = vec![0; 240];
        let mut encoder = Simple8bEncoder::new();
        encoder.encode(&values).unwrap();
        let encoded = encoder.finish();

        let decoder = Simple8bDecoder::new(encoded).unwrap();
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn test_simple8b_small_values() {
        let values: Vec<u64> = vec![0, 1, 0, 1, 1, 0, 1, 0];
        let mut encoder = Simple8bEncoder::new();
        encoder.encode(&values).unwrap();
        let encoded = encoder.finish();

        let decoder = Simple8bDecoder::new(encoded).unwrap();
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn test_simple8b_medium_values() {
        let values: Vec<u64> = vec![100, 200, 300, 400];
        let mut encoder = Simple8bEncoder::new();
        encoder.encode(&values).unwrap();
        let encoded = encoder.finish();

        let decoder = Simple8bDecoder::new(encoded).unwrap();
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn test_simple8b_same_value_rle() {
        let values: Vec<u64> = vec![42; 120];
        let mut encoder = Simple8bEncoder::new();
        encoder.encode(&values).unwrap();
        let encoded = encoder.finish();

        let decoder = Simple8bDecoder::new(encoded).unwrap();
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn test_simple8b_30bit_values() {
        let values: Vec<u64> = vec![(1u64 << 29), (1u64 << 29) + 1];
        let mut encoder = Simple8bEncoder::new();
        encoder.encode(&values).unwrap();
        let encoded = encoder.finish();

        let decoder = Simple8bDecoder::new(encoded).unwrap();
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn test_simple8b_compression_ratio() {
        let values: Vec<u64> = vec![1; 60];
        let mut encoder = Simple8bEncoder::new();
        encoder.encode(&values).unwrap();
        let encoded = encoder.finish();

        let original_size = values.len() * 8;
        let compressed_size = encoded.len();
        let ratio = original_size as f64 / compressed_size as f64;
        assert!(
            ratio > 4.0,
            "compression ratio should be > 4:1, got {:.1}:1",
            ratio
        );
    }

    #[test]
    fn test_simple8b_mixed_values() {
        let values: Vec<u64> = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 15, 16, 31, 32];
        let mut encoder = Simple8bEncoder::new();
        encoder.encode(&values).unwrap();
        let encoded = encoder.finish();

        let decoder = Simple8bDecoder::new(encoded).unwrap();
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn test_simple8b_empty() {
        let values: Vec<u64> = vec![];
        let mut encoder = Simple8bEncoder::new();
        encoder.encode(&values).unwrap();
        let encoded = encoder.finish();

        let decoder = Simple8bDecoder::new(encoded).unwrap();
        let decoded = decoder.decode_all().unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_simple8b_large_batch() {
        let mut values = Vec::new();
        for i in 0..500 {
            values.push((i % 16) as u64);
        }
        let mut encoder = Simple8bEncoder::new();
        encoder.encode(&values).unwrap();
        let encoded = encoder.finish();

        let decoder = Simple8bDecoder::new(encoded).unwrap();
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded, values);
    }
}
