use tsdb_types::model::{DataPoint, Timestamp};
use std::hash::{DefaultHasher, Hasher, Hash};

pub const BLOCK_DURATION_SECS: u64 = 30;
pub const BLOCK_DURATION_MICROS: u64 = BLOCK_DURATION_SECS * 1_000_000;
pub const SEPARATOR: u8 = b'|';
pub const QUALIFIER_SEPARATOR: u8 = b':';

#[derive(Debug, Clone)]
pub struct RowKey {
    pub measurement: String,
    pub tags_hash: u64,
    pub block_start_timestamp: Timestamp,
}

impl RowKey {
    pub fn from_data_point(dp: &DataPoint) -> Self {
        let tags_hash = compute_tags_hash(&dp.tags);
        let block_start = align_to_block_start(dp.timestamp);
        Self {
            measurement: dp.measurement.clone(),
            tags_hash,
            block_start_timestamp: block_start,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(
            self.measurement.len() + 8 + 8 + 2,
        );
        buf.extend_from_slice(self.measurement.as_bytes());
        buf.push(SEPARATOR);
        buf.extend_from_slice(&self.tags_hash.to_be_bytes());
        buf.push(SEPARATOR);
        buf.extend_from_slice(&self.block_start_timestamp.to_be_bytes());
        buf
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        let sep1 = data.iter().position(|&b| b == SEPARATOR)?;
        let rest = &data[sep1 + 1..];
        let sep2 = rest.iter().position(|&b| b == SEPARATOR)?;

        let measurement = String::from_utf8_lossy(&data[..sep1]).to_string();
        let tags_hash = u64::from_be_bytes(rest[..sep2].try_into().ok()?);
        let block_start_timestamp = i64::from_be_bytes(rest[sep2 + 1..].try_into().ok()?);

        Some(Self {
            measurement,
            tags_hash,
            block_start_timestamp,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Qualifier {
    pub field_name: String,
    pub microsecond_offset: u32,
}

impl Qualifier {
    pub fn new(field_name: impl Into<String>, timestamp: Timestamp, block_start: Timestamp) -> Self {
        let offset_micros = (timestamp - block_start) as u64;
        Self {
            field_name: field_name.into(),
            microsecond_offset: offset_micros as u32,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.field_name.len() + 4 + 1);
        buf.extend_from_slice(self.field_name.as_bytes());
        buf.push(QUALIFIER_SEPARATOR);
        buf.extend_from_slice(&self.microsecond_offset.to_be_bytes());
        buf
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        let sep = data.iter().position(|&b| b == QUALIFIER_SEPARATOR)?;
        let field_name = String::from_utf8_lossy(&data[..sep]).to_string();
        let microsecond_offset = u32::from_be_bytes(data[sep + 1..].try_into().ok()?);
        Some(Self {
            field_name,
            microsecond_offset,
        })
    }
}

pub fn align_to_block_start(timestamp_micros: Timestamp) -> Timestamp {
    let block_micros = BLOCK_DURATION_MICROS as Timestamp;
    (timestamp_micros / block_micros) * block_micros
}

pub fn compute_tags_hash(tags: &tsdb_types::model::Tags) -> u64 {
    let mut hasher = DefaultHasher::new();
    for (k, v) in tags {
        Hash::hash(&k, &mut hasher);
        Hash::hash(&v, &mut hasher);
    }
    hasher.finish()
}

pub fn timestamp_to_cf_name(timestamp_micros: Timestamp) -> String {
    let secs = timestamp_micros / 1_000_000;
    let dt = chrono::DateTime::from_timestamp(secs, 0).unwrap_or_default();
    format!("data_{}", dt.format("%Y%m%d"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsdb_types::model::FieldValue;
    use std::collections::BTreeMap;

    #[test]
    fn test_block_alignment() {
        assert_eq!(align_to_block_start(0), 0);
        assert_eq!(align_to_block_start(15_000_000), 0);
        assert_eq!(align_to_block_start(30_000_000), 30_000_000);
        assert_eq!(align_to_block_start(45_000_000), 30_000_000);
        assert_eq!(align_to_block_start(60_000_000), 60_000_000);
    }

    #[test]
    fn test_rowkey_encode_decode() {
        let dp = DataPoint::new("cpu", 45_000_000)
            .with_tag("host", "server01")
            .with_tag("region", "us-west");
        let rk = RowKey::from_data_point(&dp);
        let encoded = rk.encode();
        let decoded = RowKey::decode(&encoded).unwrap();
        assert_eq!(decoded.measurement, "cpu");
        assert_eq!(decoded.tags_hash, rk.tags_hash);
        assert_eq!(decoded.block_start_timestamp, 30_000_000);
    }

    #[test]
    fn test_qualifier_encode_decode() {
        let q = Qualifier::new("usage", 45_000_123, 30_000_000);
        let encoded = q.encode();
        let decoded = Qualifier::decode(&encoded).unwrap();
        assert_eq!(decoded.field_name, "usage");
        assert_eq!(decoded.microsecond_offset, 15_000_123);
    }

    #[test]
    fn test_tags_hash_deterministic() {
        let mut tags1 = BTreeMap::new();
        tags1.insert("host".to_string(), "server01".to_string());
        tags1.insert("region".to_string(), "us-west".to_string());

        let mut tags2 = BTreeMap::new();
        tags2.insert("region".to_string(), "us-west".to_string());
        tags2.insert("host".to_string(), "server01".to_string());

        assert_eq!(compute_tags_hash(&tags1), compute_tags_hash(&tags2));
    }

    #[test]
    fn test_cf_name() {
        let ts = 1704067200_000_000i64;
        let name = timestamp_to_cf_name(ts);
        assert!(name.starts_with("data_"));
    }
}
