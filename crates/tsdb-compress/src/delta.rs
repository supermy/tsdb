use crate::error::{CompressError, CompressResult};

pub struct DeltaEncoder {
    first_timestamp: i64,
    last_timestamp: i64,
    last_delta: i64,
    initialized: bool,
    encoded: Vec<u8>,
}

impl DeltaEncoder {
    pub fn new() -> Self {
        Self {
            first_timestamp: 0,
            last_timestamp: 0,
            last_delta: 0,
            initialized: false,
            encoded: Vec::new(),
        }
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

        self.encode_varint(delta_of_delta);

        self.last_delta = delta;
        self.last_timestamp = timestamp;
        Ok(())
    }

    pub fn finish(self) -> Vec<u8> {
        self.encoded
    }

    fn encode_varint(&mut self, val: i64) {
        let zigzag = Self::zigzag_encode(val);
        Self::encode_unsigned_varint(&mut self.encoded, zigzag);
    }

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
}

impl DeltaDecoder {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            last_timestamp: 0,
            last_delta: 0,
            initialized: false,
            data,
            pos: 0,
        }
    }

    pub fn decode_next(&mut self) -> CompressResult<Option<i64>> {
        if !self.initialized {
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

        if self.pos >= self.data.len() {
            return Ok(None);
        }

        let zigzag = Self::decode_unsigned_varint(&self.data, &mut self.pos)?;
        let dod = Self::zigzag_decode(zigzag);
        let delta = self.last_delta + dod;
        let timestamp = self.last_timestamp + delta;

        self.last_delta = delta;
        self.last_timestamp = timestamp;

        Ok(Some(timestamp))
    }

    pub fn decode_all(mut self) -> CompressResult<Vec<i64>> {
        let mut results = Vec::new();
        while let Some(ts) = self.decode_next()? {
            results.push(ts);
        }
        Ok(results)
    }

    fn zigzag_decode(n: u64) -> i64 {
        ((n >> 1) as i64) ^ -((n & 1) as i64)
    }

    fn decode_unsigned_varint(data: &[u8], pos: &mut usize) -> CompressResult<u64> {
        let mut val: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            if *pos >= data.len() {
                return Err(CompressError::Decode("unexpected end of varint data".into()));
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
    fn test_zigzag() {
        assert_eq!(DeltaEncoder::zigzag_encode(0), 0);
        assert_eq!(DeltaEncoder::zigzag_encode(-1), 1);
        assert_eq!(DeltaEncoder::zigzag_encode(1), 2);
        assert_eq!(DeltaEncoder::zigzag_encode(-2), 3);
        assert_eq!(DeltaDecoder::zigzag_decode(0), 0);
        assert_eq!(DeltaDecoder::zigzag_decode(1), -1);
        assert_eq!(DeltaDecoder::zigzag_decode(2), 1);
        assert_eq!(DeltaDecoder::zigzag_decode(3), -2);
    }
}
