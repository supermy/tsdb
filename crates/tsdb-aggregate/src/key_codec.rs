//! 汇总 Key 编解码模块 - Aggregation Key Codec
//!
//! 轻度汇总数据采用 `business|dimension|timestamp` 格式的 key 编码，
//! 相同业务+维度的 key 共享前缀字节，利用 RocksDB 的前缀压缩特性
//! 减少存储空间。
//!
//! ## Key 格式
//!
//! ```text
//! [business_len:1B] [business] | [dimension_len:1B] [dimension] | [timestamp:8B BE]
//! ```
//!
//! ## 压缩优势
//!
//! RocksDB 默认对前 16 字节做前缀压缩，同一业务+维度的 key 共享
//! `business|dimension` 前缀，只需存储差异部分（timestamp）。

const SEPARATOR: u8 = b'|';

#[derive(Debug, Clone, PartialEq)]
pub struct AggregationKey {
    pub business: String,
    pub dimension: String,
    pub timestamp: i64,
}

impl AggregationKey {
    pub fn new(business: &str, dimension: &str, timestamp: i64) -> Self {
        Self {
            business: business.to_string(),
            dimension: dimension.to_string(),
            timestamp,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + self.business.len() + 1 + self.dimension.len() + 8);

        buf.push(self.business.len() as u8);
        buf.extend_from_slice(self.business.as_bytes());
        buf.push(SEPARATOR);
        buf.push(self.dimension.len() as u8);
        buf.extend_from_slice(self.dimension.as_bytes());
        buf.push(SEPARATOR);
        buf.extend_from_slice(&self.timestamp.to_be_bytes());

        buf
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 4 {
            return None;
        }

        let business_len = data[0] as usize;
        if data.len() < 1 + business_len + 1 {
            return None;
        }
        let business = String::from_utf8_lossy(&data[1..1 + business_len]).to_string();

        let sep1_pos = 1 + business_len;
        if data[sep1_pos] != SEPARATOR {
            return None;
        }

        let dim_start = sep1_pos + 1;
        if data.len() < dim_start + 1 {
            return None;
        }
        let dimension_len = data[dim_start] as usize;
        if data.len() < dim_start + 1 + dimension_len + 1 {
            return None;
        }
        let dimension =
            String::from_utf8_lossy(&data[dim_start + 1..dim_start + 1 + dimension_len])
                .to_string();

        let sep2_pos = dim_start + 1 + dimension_len;
        if data[sep2_pos] != SEPARATOR {
            return None;
        }

        let ts_start = sep2_pos + 1;
        if data.len() < ts_start + 8 {
            return None;
        }
        let timestamp = i64::from_be_bytes(data[ts_start..ts_start + 8].try_into().ok()?);

        Some(AggregationKey {
            business,
            dimension,
            timestamp,
        })
    }

    pub fn prefix_for_business_dimension(business: &str, dimension: &str) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + business.len() + 1 + 1 + dimension.len() + 1);
        buf.push(business.len() as u8);
        buf.extend_from_slice(business.as_bytes());
        buf.push(SEPARATOR);
        buf.push(dimension.len() as u8);
        buf.extend_from_slice(dimension.as_bytes());
        buf.push(SEPARATOR);
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_roundtrip() {
        let key = AggregationKey::new("stocks", "hour", 1710000000);
        let encoded = key.encode();
        let decoded = AggregationKey::decode(&encoded).unwrap();
        assert_eq!(decoded, key);
    }

    #[test]
    fn test_prefix_generation() {
        let prefix = AggregationKey::prefix_for_business_dimension("iot", "day");
        let key = AggregationKey::new("iot", "day", 1710000000);
        let encoded = key.encode();
        assert!(encoded.starts_with(&prefix));
    }

    #[test]
    fn test_different_business_different_prefix() {
        let prefix1 = AggregationKey::prefix_for_business_dimension("stocks", "hour");
        let prefix2 = AggregationKey::prefix_for_business_dimension("iot", "hour");
        assert_ne!(prefix1, prefix2);
    }

    #[test]
    fn test_same_business_different_dimension() {
        let prefix1 = AggregationKey::prefix_for_business_dimension("stocks", "hour");
        let prefix2 = AggregationKey::prefix_for_business_dimension("stocks", "day");
        assert_ne!(prefix1, prefix2);
    }
}
