//! Series Key 布隆过滤器模块 - Series Key Bloom Filter Module
//!
//! 布隆过滤器用于快速判断 series key 是否可能存在于某个 SST 文件中。
//! 当过滤器返回 false 时，可以确定 key 不存在，跳过该文件。
//! 当过滤器返回 true 时，key 可能存在（有假阳性），需要进一步查找。
//!
//! ## InfluxDB 对标
//!
//! InfluxDB TSM 引擎在每个 SST 文件中嵌入布隆过滤器：
//! - 写入时：将 series key 添加到过滤器
//! - 查询时：先检查过滤器，false 则跳过该文件
//! - 压缩时：合并多个文件的过滤器
//!
//! ## 参数选择
//!
//! | 参数 | 值 | 说明 |
//! |------|-----|------|
//! | 误判率 | 0.01 (1%) | 平衡内存和准确性 |
//! | 哈希函数数 | 7 | ln(2) × m/n ≈ 7 |
//! | 位数组大小 | 动态 | 根据预期元素数量计算 |

use std::hash::{Hash, Hasher};

const MURMUR_SEEDS: [u64; 7] = [
    0x12345678, 0x9ABCDEF0, 0x13579BDF, 0x2468ACE0, 0xFDB97531, 0xCAFE1234, 0xDEADBEEF,
];

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BloomFilter {
    bits: Vec<u64>,
    num_bits: usize,
    num_hashes: usize,
    count: usize,
}

impl BloomFilter {
    pub fn new(expected_items: usize) -> Self {
        Self::with_fpp(expected_items, 0.01)
    }

    pub fn with_fpp(expected_items: usize, fpp: f64) -> Self {
        let num_bits = Self::optimal_num_bits(expected_items, fpp);
        let num_hashes = Self::optimal_num_hashes(num_bits, expected_items);
        let words = num_bits.div_ceil(64);
        Self {
            bits: vec![0u64; words],
            num_bits,
            num_hashes,
            count: 0,
        }
    }

    fn optimal_num_bits(n: usize, p: f64) -> usize {
        let m = -(n as f64) * p.ln() / (2.0_f64.ln().powi(2));
        m.ceil() as usize
    }

    fn optimal_num_hashes(m: usize, n: usize) -> usize {
        if n == 0 {
            return 1;
        }
        let k = ((m as f64) / (n as f64) * 2.0_f64.ln()).round() as usize;
        k.max(1).min(MURMUR_SEEDS.len())
    }

    pub fn insert(&mut self, key: &[u8]) {
        let hashes = self.hash_key(key);
        for &bit_idx in &hashes {
            let word_idx = bit_idx / 64;
            let bit_offset = bit_idx % 64;
            self.bits[word_idx] |= 1u64 << bit_offset;
        }
        self.count += 1;
    }

    pub fn insert_str(&mut self, key: &str) {
        self.insert(key.as_bytes());
    }

    pub fn might_contain(&self, key: &[u8]) -> bool {
        let hashes = self.hash_key(key);
        for &bit_idx in &hashes {
            let word_idx = bit_idx / 64;
            let bit_offset = bit_idx % 64;
            if self.bits[word_idx] & (1u64 << bit_offset) == 0 {
                return false;
            }
        }
        true
    }

    pub fn might_contain_str(&self, key: &str) -> bool {
        self.might_contain(key.as_bytes())
    }

    fn hash_key(&self, key: &[u8]) -> Vec<usize> {
        let mut hashes = Vec::with_capacity(self.num_hashes);
        for &seed in MURMUR_SEEDS.iter().take(self.num_hashes) {
            let hash = Self::murmur_hash(key, seed);
            hashes.push((hash as usize) % self.num_bits);
        }
        hashes
    }

    fn murmur_hash(data: &[u8], seed: u64) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        seed.hash(&mut hasher);
        data.hash(&mut hasher);
        hasher.finish()
    }

    pub fn merge(&mut self, other: &BloomFilter) {
        if self.bits.len() != other.bits.len() {
            return;
        }
        for i in 0..self.bits.len() {
            self.bits[i] |= other.bits[i];
        }
        self.count += other.count;
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn estimated_fpp(&self) -> f64 {
        if self.num_bits == 0 || self.count == 0 {
            return 0.0;
        }
        let k = self.num_hashes as f64;
        let m = self.num_bits as f64;
        let n = self.count as f64;
        (1.0 - (1.0 - 1.0 / m).powf(k * n)).powf(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bloom_filter_basic() {
        let mut bf = BloomFilter::new(1000);
        bf.insert(b"cpu|host=server1");
        bf.insert(b"cpu|host=server2");
        bf.insert(b"mem|host=server1");

        assert!(bf.might_contain(b"cpu|host=server1"));
        assert!(bf.might_contain(b"cpu|host=server2"));
        assert!(bf.might_contain(b"mem|host=server1"));
        assert!(!bf.might_contain(b"disk|host=server3"));
    }

    #[test]
    fn test_bloom_filter_str() {
        let mut bf = BloomFilter::new(100);
        bf.insert_str("cpu|host=server1");
        assert!(bf.might_contain_str("cpu|host=server1"));
        assert!(!bf.might_contain_str("mem|host=server2"));
    }

    #[test]
    fn test_bloom_filter_merge() {
        let mut bf1 = BloomFilter::new(100);
        bf1.insert(b"cpu|host=server1");

        let mut bf2 = BloomFilter::new(100);
        bf2.insert(b"mem|host=server2");

        bf1.merge(&bf2);
        assert!(bf1.might_contain(b"cpu|host=server1"));
        assert!(bf1.might_contain(b"mem|host=server2"));
    }

    #[test]
    fn test_bloom_filter_fpp() {
        let n = 10000;
        let mut bf = BloomFilter::with_fpp(n, 0.01);

        for i in 0..n {
            let key = format!("series_{}", i);
            bf.insert(key.as_bytes());
        }

        let mut false_positives = 0;
        let test_count = 100000;
        for i in n..n + test_count {
            let key = format!("series_{}", i);
            if bf.might_contain(key.as_bytes()) {
                false_positives += 1;
            }
        }

        let actual_fpp = false_positives as f64 / test_count as f64;
        assert!(
            actual_fpp < 0.05,
            "FPP should be < 5%, got {:.2}%",
            actual_fpp * 100.0
        );
    }

    #[test]
    fn test_bloom_filter_empty() {
        let bf = BloomFilter::new(100);
        assert!(bf.is_empty());
        assert!(!bf.might_contain(b"anything"));
    }

    #[test]
    fn test_bloom_filter_count() {
        let mut bf = BloomFilter::new(100);
        assert_eq!(bf.count(), 0);
        bf.insert(b"key1");
        assert_eq!(bf.count(), 1);
        bf.insert(b"key2");
        assert_eq!(bf.count(), 2);
    }
}
