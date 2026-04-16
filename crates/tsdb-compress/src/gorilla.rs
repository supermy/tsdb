//! # Gorilla XOR 浮点压缩算法
//!
//! ## 算法原理
//!
//! Gorilla 压缩是 Facebook Gorilla TSDB 的核心创新，专门针对 **浮点数时间序列** 优化：
//!
//! ```text
//! 核心思想：相邻浮点值的 XOR 差异通常很小（仅几位不同）
//!
//! 原始值:    3FF19999A... (1.6 的 IEEE 754 表示)
//! 上一个值:  3FF19999C...
//! XOR 结果: 0000000000000002  ← 仅最后几位不同！
//! ```
//!
//! ## 编码格式
//!
//! | 情况 | 编码位模式 | 说明 |
//! |------|-----------|------|
//! | 首个值 | `0` (64 bits) | 完整存储第一个 float64 |
//! | 值相同 | `0` | 仅 1 bit，表示与上一个值完全相同 |
//! | 复用前导/后导零 | `1 0` + 有效位 | 利用上一次的零位信息 |
//! | 新的零位模式 | `1 1` + 前导零(6b) + 有效长度(6b) + 有效位 | 全新编码 |
//!
//! ## 压缩效果
//!
//! 对于典型的监控数据（变化缓慢），压缩比可达 **10:1** 以上。
//!

use crate::error::{CompressError, CompressResult};

/// 每字节的位数常量
const BITS_PER_BYTE: u8 = 8;

/// Gorilla 浮点压缩编码器
///
/// 将连续的 f64 浮点值序列压缩为紧凑的二进制位流。
///
/// ## 内部状态
///
/// - `last_value`: 上一个原始值的 IEEE 754 位表示
/// - `last_leading_zeros/trailing_zeros`: 上一次 XOR 结果的前导/后导零位数
/// - `buf/current_byte/bits_used`: 位级写入缓冲区
///
/// ## 使用示例
///
/// 创建编码器后逐个调用 encode() 压缩浮点值，
/// 最后调用 finish() 获取压缩后的二进制数据。
///
#[derive(Default)]
pub struct GorillaEncoder {
    last_value: u64,
    last_leading_zeros: u8,
    last_trailing_zeros: u8,
    initialized: bool,
    count: u32,
    buf: Vec<u8>,
    current_byte: u8,
    bits_used: u8,
}

impl GorillaEncoder {
    pub fn new() -> Self {
        Self::default()
    }
}

impl GorillaEncoder {
    /// 编码单个 f64 浮点值到位流中
    /// ## 编码逻辑
    ///
    /// 1. **首个值**：直接写入完整的 64 位 IEEE 754 表示
    /// 2. **XOR = 0**：值与上一个完全相同 → 写入单个 `0` bit（极致压缩！）
    /// 3. **XOR ≠ 0 且可复用零位信息**：
    ///    写入 `1 0` + 有效位（省略前导/后导零的编码开销）
    /// 4. **XOR ≠ 0 且需新零位信息**：
    ///    写入 `1 1` + 前导零数(6bit) + 有效位长度(6bit) + 有效位
    ///
    /// # 参数
    /// - `value`: 待编码的 f64 浮点值
    pub fn encode(&mut self, value: f64) -> CompressResult<()> {
        let bits = value.to_bits();
        if !self.initialized {
            self.last_value = bits;
            self.initialized = true;
            self.write_bits(bits, 64);
            self.count = 1;
            return Ok(());
        }

        let xor = bits ^ self.last_value;
        if xor == 0 {
            self.write_bit(false);
        } else {
            self.write_bit(true);
            let leading = xor.leading_zeros() as u8;
            let trailing = xor.trailing_zeros() as u8;

            let can_reuse = self.last_leading_zeros > 0
                && leading >= self.last_leading_zeros
                && trailing >= self.last_trailing_zeros;

            if can_reuse {
                self.write_bit(false);
                let meaningful_bits = 64 - self.last_leading_zeros - self.last_trailing_zeros;
                self.write_bits(xor >> self.last_trailing_zeros, meaningful_bits as u32);
            } else {
                self.write_bit(true);
                if leading > 31 {
                    self.write_bits(31, 6);
                } else {
                    self.write_bits(leading as u64, 6);
                }
                let meaningful_bits = 64 - leading - trailing;
                self.write_bits((meaningful_bits - 1) as u64, 6);
                if meaningful_bits > 0 {
                    self.write_bits(xor >> trailing, meaningful_bits as u32);
                }
                self.last_leading_zeros = leading;
                self.last_trailing_zeros = trailing;
            }
        }

        self.last_value = bits;
        self.count += 1;
        Ok(())
    }

    /// 结束编码并返回压缩后的二进制数据
    ///
    /// 输出格式：
    /// ```text
    /// [4 字节: 值计数 (Big-Endian u32)] [N 字节: 位流数据]
    /// ```
    ///
    /// # 返回
    /// 完整的压缩二进制数据（可直接传给 GorillaDecoder 解码）
    pub fn finish(mut self) -> Vec<u8> {
        if self.bits_used > 0 {
            self.buf.push(self.current_byte);
        }
        let mut result = Vec::with_capacity(4 + self.buf.len());
        result.extend_from_slice(&self.count.to_be_bytes());
        result.extend_from_slice(&self.buf);
        result
    }

    /// 向位流写入单个 bit
    ///
    /// 从最高位（MSB）开始填充当前字节，
    /// 字节写满后自动推入 buf 并重置。
    fn write_bit(&mut self, bit: bool) {
        if bit {
            self.current_byte |= 1 << (BITS_PER_BYTE - 1 - self.bits_used);
        }
        self.bits_used += 1;
        if self.bits_used == BITS_PER_BYTE {
            self.buf.push(self.current_byte);
            self.current_byte = 0;
            self.bits_used = 0;
        }
    }

    /// 向位流写入多个 bits（最多 64 位）
    ///
    /// 从高位到低位逐批写入，自动处理跨字节边界的情况。
    ///
    /// # 参数
    /// - `value`: 待写入的位数据（从高位开始取）
    /// - `count`: 需要写入的位数
    fn write_bits(&mut self, value: u64, mut count: u32) {
        while count > 0 {
            let bits_available = BITS_PER_BYTE - self.bits_used;
            let bits_to_write = std::cmp::min(bits_available as u32, count);
            let shift = count - bits_to_write;
            let mask = if bits_to_write == 64 {
                u64::MAX
            } else {
                ((1u64 << bits_to_write) - 1) << shift
            };
            let bits = (value & mask) >> shift;

            self.current_byte |= (bits as u8) << (bits_available - bits_to_write as u8);
            self.bits_used += bits_to_write as u8;
            count -= bits_to_write;

            if self.bits_used == BITS_PER_BYTE {
                self.buf.push(self.current_byte);
                self.current_byte = 0;
                self.bits_used = 0;
            }
        }
    }
}

/// Gorilla 浮点压缩解码器
///
/// 从 GorillaEncoder 产生的二进制位流中逐个还原 f64 浮点值。
///
/// ## 解码流程（编码的逆过程）
///
/// 1. 读取首值（完整 64 位）
/// 2. 读控制位：`0` → 值不变；`1` → 有差异
/// 3. 若有差异，读复用标志：`0` → 复用零位信息；`1` → 读取新的零位信息
/// 4. 读取有效位并与上一个值 XOR 还原
pub struct GorillaDecoder {
    /// 压缩数据（去掉 4 字节头后的纯位流部分）
    data: Vec<u8>,
    /// 当前读取位置（字节索引）
    byte_pos: usize,
    /// 当前字节内的位偏移（0~7，从 MSB 开始）
    bit_pos: u8,
    /// 上一个解码出的 IEEE 754 位值
    last_value: u64,
    /// 上一次使用的前导零位数
    last_leading_zeros: u8,
    /// 上一次使用的后导零位数
    last_trailing_zeros: u8,
    /// 是否已完成首值初始化
    initialized: bool,
    /// 剩余待解码的值数量
    remaining: u32,
}

impl GorillaDecoder {
    /// 从压缩数据创建解码器实例
    ///
    /// # 参数
    /// - `data`: GorillaEncoder.finish() 输出的完整压缩数据
    ///
    /// # 返回
    /// - `Ok(GorillaDecoder)`: 已初始化的解码器
    /// - `Err(CompressError::Decode)`: 数据太短或格式无效
    pub fn new(data: Vec<u8>) -> CompressResult<Self> {
        if data.len() < 4 {
            return Err(CompressError::Decode("data too short".into()));
        }
        let count = u32::from_be_bytes(data[0..4].try_into().map_err(|_| CompressError::Decode("invalid count".into()))?);
        Ok(Self {
            data: data[4..].to_vec(),
            byte_pos: 0,
            bit_pos: 0,
            last_value: 0,
            last_leading_zeros: 0,
            last_trailing_zeros: 0,
            initialized: false,
            remaining: count,
        })
    }

    /// 解码下一个浮点值
    ///
    /// 按照 Gorilla 编码格式的逆过程逐步解析位流。
    /// 每次调用消耗若干 bit，返回一个还原的 f64 值。
    ///
    /// # 返回
    /// - `Ok(Some(f64))`: 成功解码一个值
    /// - `Ok(None)`: 已无更多数据可解码
    /// - `Err(CompressError::Decode)`: 位流数据损坏或格式错误
    pub fn decode_next(&mut self) -> CompressResult<Option<f64>> {
        if self.remaining == 0 {
            return Ok(None);
        }

        if !self.initialized {
            if self.data.len() < 8 {
                return Ok(None);
            }
            let bits = self.read_bits(64)?;
            self.last_value = bits;
            self.initialized = true;
            self.remaining -= 1;
            return Ok(Some(f64::from_bits(bits)));
        }

        if self.byte_pos >= self.data.len() {
            return Ok(None);
        }

        let control_bit = self.read_bit()?;
        if !control_bit {
            self.remaining -= 1;
            return Ok(Some(f64::from_bits(self.last_value)));
        }

        let reuse_meaningful = self.read_bit()?;
        if !reuse_meaningful {
            let meaningful_bits = 64 - self.last_leading_zeros - self.last_trailing_zeros;
            if meaningful_bits == 0 {
                self.remaining -= 1;
                return Ok(Some(f64::from_bits(self.last_value)));
            }
            let value_bits = self.read_bits(meaningful_bits as u32)?;
            let xor = value_bits << self.last_trailing_zeros;
            self.last_value ^= xor;
            self.remaining -= 1;
            return Ok(Some(f64::from_bits(self.last_value)));
        }

        let leading = self.read_bits(6)? as u8;
        if leading > 31 {
            return Err(CompressError::Decode(format!("invalid leading zeros: {}", leading)));
        }
        let meaningful_bits = self.read_bits(6)? as u8 + 1;
        let trailing = 64 - leading - meaningful_bits;

        let value_bits = self.read_bits(meaningful_bits as u32)?;
        let xor = if trailing > 0 {
            value_bits << trailing
        } else {
            value_bits
        };

        self.last_leading_zeros = leading;
        self.last_trailing_zeros = trailing;
        self.last_value ^= xor;
        self.remaining -= 1;

        Ok(Some(f64::from_bits(self.last_value)))
    }

    /// 一次性解码所有剩余的浮点值
    ///
    /// 便捷方法，循环调用 decode_next() 直到返回 None。
    ///
    /// # 返回
    /// 所有解码出的 f64 值的向量
    pub fn decode_all(mut self) -> CompressResult<Vec<f64>> {
        let mut results = Vec::new();
        while let Some(v) = self.decode_next()? {
            results.push(v);
        }
        Ok(results)
    }

    /// 从位流中读取单个 bit
    ///
    /// 从当前 byte_pos 和 bit_pos 位置提取 1 个 bit，
    /// 并推进读取位置指针。跨字节时自动处理。
    fn read_bit(&mut self) -> CompressResult<bool> {
        if self.byte_pos >= self.data.len() {
            return Err(CompressError::Decode("unexpected end of data".into()));
        }
        let bit = (self.data[self.byte_pos] >> (BITS_PER_BYTE - 1 - self.bit_pos)) & 1;
        self.bit_pos += 1;
        if self.bit_pos == BITS_PER_BYTE {
            self.byte_pos += 1;
            self.bit_pos = 0;
        }
        Ok(bit == 1)
    }

    /// 从位流中读取指定位数的值
    ///
    /// 逐 bit 调用 read_bit() 并组装成 u64 整数。
    ///
    /// # 参数
    /// - `count`: 需要读取的位数
    fn read_bits(&mut self, count: u32) -> CompressResult<u64> {
        let mut result = 0u64;
        for _ in 0..count {
            result = (result << 1) | if self.read_bit()? { 1 } else { 0 };
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gorilla_encode_decode() {
        let values = vec![1.0, 1.1, 1.2, 1.3, 1.4, 1.5];
        let mut encoder = GorillaEncoder::new();
        for &v in &values {
            encoder.encode(v).unwrap();
        }
        let encoded = encoder.finish();

        let decoder = GorillaDecoder::new(encoded).unwrap();
        let decoded = decoder.decode_all().unwrap();

        assert_eq!(decoded.len(), values.len());
        for (orig, dec) in values.iter().zip(decoded.iter()) {
            assert!((orig - dec).abs() < f64::EPSILON, "expected {} got {}", orig, dec);
        }
    }

    #[test]
    fn test_gorilla_constant_values() {
        let values = vec![std::f64::consts::PI; 100];
        let mut encoder = GorillaEncoder::new();
        for &v in &values {
            encoder.encode(v).unwrap();
        }
        let encoded = encoder.finish();

        let decoder = GorillaDecoder::new(encoded).unwrap();
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded.len(), values.len());
        for v in &decoded {
            assert!((v - std::f64::consts::PI).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn test_gorilla_simple() {
        let values = vec![1.0, 2.0, 3.0];
        let mut encoder = GorillaEncoder::new();
        for &v in &values {
            encoder.encode(v).unwrap();
        }
        let encoded = encoder.finish();

        let decoder = GorillaDecoder::new(encoded).unwrap();
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded.len(), 3);
    }
}
