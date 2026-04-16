//! 时序感知比较器模块 - Time-Series Aware Comparator
//!
//! 自定义 RocksDB Comparator，按 measurement → tags_hash → block_timestamp 排序，
//! 确保同一 Series 的数据物理相邻，提升范围扫描局部性。
//!
//! ## 排序规则
//!
//! Key 格式: `measurement|tags_hash|block_timestamp`
//!
//! 比较顺序:
//! 1. measurement (字典序)
//! 2. tags_hash (大端序数值比较)
//! 3. block_timestamp (大端序数值比较)

use std::cmp::Ordering;

const SEPARATOR: u8 = b'|';

pub fn tsdb_compare(a: &[u8], b: &[u8]) -> Ordering {
    let a_parts = split_rowkey(a);
    let b_parts = split_rowkey(b);

    match a_parts.measurement.cmp(b_parts.measurement) {
        Ordering::Equal => {}
        ord => return ord,
    }

    match a_parts.tags_hash.cmp(&b_parts.tags_hash) {
        Ordering::Equal => {}
        ord => return ord,
    }

    a_parts.block_ts.cmp(&b_parts.block_ts)
}

struct RowKeyParts<'a> {
    measurement: &'a [u8],
    tags_hash: u64,
    block_ts: i64,
}

fn split_rowkey(data: &[u8]) -> RowKeyParts<'_> {
    let sep1 = data.iter().position(|&b| b == SEPARATOR);
    let measurement;
    let rest;

    if let Some(pos) = sep1 {
        measurement = &data[..pos];
        rest = &data[pos + 1..];
    } else {
        measurement = data;
        return RowKeyParts {
            measurement,
            tags_hash: 0,
            block_ts: 0,
        };
    }

    let sep2 = rest.iter().position(|&b| b == SEPARATOR);
    let tags_hash;
    let block_ts_rest;

    if let Some(pos) = sep2 {
        let hash_bytes = &rest[..pos];
        tags_hash = if hash_bytes.len() == 8 {
            u64::from_be_bytes(hash_bytes.try_into().unwrap_or([0; 8]))
        } else {
            0
        };
        block_ts_rest = &rest[pos + 1..];
    } else {
        return RowKeyParts {
            measurement,
            tags_hash: 0,
            block_ts: 0,
        };
    }

    let block_ts = if block_ts_rest.len() == 8 {
        i64::from_be_bytes(block_ts_rest.try_into().unwrap_or([0; 8]))
    } else {
        0
    };

    RowKeyParts {
        measurement,
        tags_hash,
        block_ts,
    }
}

pub fn register_comparator(opts: &mut rocksdb::Options) {
    opts.set_comparator("tsdb.comparator", Box::new(tsdb_compare));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_key(measurement: &str, tags_hash: u64, block_ts: i64) -> Vec<u8> {
        let mut buf = measurement.as_bytes().to_vec();
        buf.push(SEPARATOR);
        buf.extend_from_slice(&tags_hash.to_be_bytes());
        buf.push(SEPARATOR);
        buf.extend_from_slice(&block_ts.to_be_bytes());
        buf
    }

    #[test]
    fn test_compare_same_measurement() {
        let a = make_key("cpu", 100, 1000);
        let b = make_key("cpu", 100, 2000);
        assert_eq!(tsdb_compare(&a, &b), Ordering::Less);
    }

    #[test]
    fn test_compare_different_measurement() {
        let a = make_key("cpu", 100, 1000);
        let b = make_key("mem", 100, 1000);
        assert_eq!(tsdb_compare(&a, &b), Ordering::Less);
    }

    #[test]
    fn test_compare_different_tags_hash() {
        let a = make_key("cpu", 100, 1000);
        let b = make_key("cpu", 200, 1000);
        assert_eq!(tsdb_compare(&a, &b), Ordering::Less);
    }

    #[test]
    fn test_compare_equal() {
        let a = make_key("cpu", 100, 1000);
        let b = make_key("cpu", 100, 1000);
        assert_eq!(tsdb_compare(&a, &b), Ordering::Equal);
    }

    #[test]
    fn test_compare_with_qualifier_key() {
        let mut a = make_key("cpu", 100, 1000);
        a.push(0x00);
        a.extend_from_slice(b"usage:15000");

        let mut b = make_key("cpu", 100, 1000);
        b.push(0x00);
        b.extend_from_slice(b"system:15000");

        assert_eq!(tsdb_compare(&a, &b), Ordering::Equal);
    }
}
