use crate::error::{CompressError, CompressResult};

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

const BITS_PER_BYTE: u8 = 8;

impl GorillaEncoder {
    pub fn new() -> Self {
        Self {
            last_value: 0,
            last_leading_zeros: 0,
            last_trailing_zeros: 0,
            initialized: false,
            count: 0,
            buf: Vec::new(),
            current_byte: 0,
            bits_used: 0,
        }
    }

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

    pub fn finish(mut self) -> Vec<u8> {
        if self.bits_used > 0 {
            self.buf.push(self.current_byte);
        }
        let mut result = Vec::with_capacity(4 + self.buf.len());
        result.extend_from_slice(&self.count.to_be_bytes());
        result.extend_from_slice(&self.buf);
        result
    }

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

    fn write_bits(&mut self, mut value: u64, mut count: u32) {
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

pub struct GorillaDecoder {
    data: Vec<u8>,
    byte_pos: usize,
    bit_pos: u8,
    last_value: u64,
    last_leading_zeros: u8,
    last_trailing_zeros: u8,
    initialized: bool,
    remaining: u32,
}

impl GorillaDecoder {
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

    pub fn decode_all(mut self) -> CompressResult<Vec<f64>> {
        let mut results = Vec::new();
        while let Some(v) = self.decode_next()? {
            results.push(v);
        }
        Ok(results)
    }

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
        let values = vec![3.14; 100];
        let mut encoder = GorillaEncoder::new();
        for &v in &values {
            encoder.encode(v).unwrap();
        }
        let encoded = encoder.finish();

        let decoder = GorillaDecoder::new(encoded).unwrap();
        let decoded = decoder.decode_all().unwrap();
        assert_eq!(decoded.len(), values.len());
        for v in &decoded {
            assert!((v - 3.14).abs() < f64::EPSILON);
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
