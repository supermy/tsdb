//! # 块编解码器（Block Codec）— 数据块压缩/解压
//!
//! ## 架构设计
//!
//! BlockCodec 是 TSDB 压缩子系统的顶层调度器，将一个 **DataBlock**（原始数据块）
//! 按字段类型分派到最合适的压缩算法：
//!
//! ```text
//! DataBlock (原始)
//! ├── timestamps ──────► DeltaEncoder (Delta-of-Delta + ZigZag + Varint)
//! ├── float 字段   ────► GorillaEncoder (XOR 浮点压缩)
//! ├── int 字段     ────► Big-Endian 原始存储 (8B/值)
//! ├── string 字段  ────► DictionaryEncoder (字典编码)
//! └── bool 字段    ────► 位打包 (8 值/字节)
//!       │
//!       ▼
//! CompressedBlock (压缩后，可序列化持久化)
//! ```
//!
//! ## 类型分派策略
//!
//! | 字段类型 | 压缩算法 | 典型压缩比 |
//! |---------|----------|-----------|
//! | i64 时间戳 | Delta-of-Delta | ~10:1 |
//! | f64 测量值 | Gorilla XOR | ~5-15:1 |
//! | i64 计数器 | 原始存储 | 1:1 (已紧凑) |
//! | String 标签 | Dictionary | ~3-10:1 |
//! | Boolean 标志 | Bit-packing | 8:1 |
//!

use crate::delta::{DeltaDecoder, DeltaEncoder};
use crate::dictionary::{DictionaryDecoder, DictionaryEncoder};
use crate::error::{CompressError, CompressResult};
use crate::gorilla::{GorillaDecoder, GorillaEncoder};
use std::collections::HashMap;
use tsdb_types::model::FieldValue;

/// 块编解码器 trait — 定义压缩/解压的统一接口
///
/// 实现此 trait 可支持不同的压缩策略（如未来可加入 ZSTD、LZ4 等）。
pub trait Codec {
    /// 将原始数据块压缩为 CompressedBlock
    fn compress_block(&self, block: &DataBlock) -> CompressResult<CompressedBlock>;
    /// 将 CompressedBlock 解压还原为原始 DataBlock
    fn decompress_block(&self, compressed: &CompressedBlock) -> CompressResult<DataBlock>;
}

/// 原始数据块 — 未压缩的时间序列数据集合
///
/// 包含一个时间戳向量和多个命名字段向量，每个字段向量内的值类型一致。
#[derive(Debug, Clone)]
pub struct DataBlock {
    /// 微秒级时间戳向量（单调递增）
    pub timestamps: Vec<i64>,
    /// 字段名 → 字段值向量的映射
    pub fields: HashMap<String, Vec<FieldValue>>,
}

/// 压缩后的数据块 — 各字段独立压缩的二进制数据
///
/// 实现了 Serialize/Deserialize 以支持通过 bincode 序列化到 RocksDB。
/// 各字段按类型分别存储在对应的 HashMap 中。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompressedBlock {
    /// Delta 编码后的时间戳二进制数据
    pub timestamps: Vec<u8>,
    /// float 字段名 → Gorilla 编码后的二进制数据
    pub float_fields: HashMap<String, Vec<u8>>,
    /// int/bool 字段名 → 原始或位打包后的二进制数据
    pub int_fields: HashMap<String, Vec<u8>>,
    /// string 字段名 → 字典 ID 序列的二进制数据
    pub string_fields: HashMap<String, Vec<u8>>,
    /// string 字段名 → 字典条目的二进制数据
    pub dictionaries: HashMap<String, Vec<u8>>,
    /// 数据行数（用于解压时预分配空间）
    pub row_count: usize,
}

/// 默认块编解码器实现 — 使用 TSDB 全套压缩算法组合
///
/// ## compress_block 流程
///
/// 1. 时间戳 → DeltaEncoder（Delta-of-Delta + ZigZag + Varint）
/// 2. 遍历每个字段，根据首值类型选择编码器：
///    - Float → GorillaEncoder（XOR 压缩）
///    - Integer → 原始 Big-Endian 存储
///    - String → DictionaryEncoder（字典编码）+ ID 序列
///    - Boolean → 位打包（每字节存储 8 个布尔值）
///
/// ## decompress_block 流程（压缩的逆过程）
///
/// 1. DeltaDecoder 还原时间戳向量
/// 2. GorillaDecoder 还原各 float 字段
/// 3. 按 8 字节/chunk 还原 int 字段
/// 4. DictionaryDecoder + ID 序列还原 string 字段
pub struct BlockCodec;

impl Codec for BlockCodec {
    /// 将原始数据块压缩为 CompressedBlock
    ///
    /// # 参数
    /// - `block`: 待压缩的原始数据块
    ///
    /// # 返回
    /// - `Ok(CompressedBlock)`: 压缩后的数据块（可直接序列化）
    /// - `Err(CompressError)`: 编码过程中发生错误
    fn compress_block(&self, block: &DataBlock) -> CompressResult<CompressedBlock> {
        let mut ts_encoder = DeltaEncoder::new();
        for &ts in &block.timestamps {
            ts_encoder.encode(ts)?;
        }

        let mut float_fields = HashMap::new();
        let mut int_fields = HashMap::new();
        let mut string_fields = HashMap::new();
        let mut dictionaries = HashMap::new();

        for (field_name, values) in &block.fields {
            if values.is_empty() {
                continue;
            }

            match &values[0] {
                FieldValue::Float(_) => {
                    let mut encoder = GorillaEncoder::new();
                    for v in values {
                        if let Some(f) = v.as_f64() {
                            encoder.encode(f)?;
                        }
                    }
                    float_fields.insert(field_name.clone(), encoder.finish());
                }
                FieldValue::Integer(_) => {
                    let mut buf = Vec::new();
                    for v in values {
                        if let Some(i) = v.as_i64() {
                            buf.extend_from_slice(&i.to_be_bytes());
                        }
                    }
                    int_fields.insert(field_name.clone(), buf);
                }
                FieldValue::String(_) => {
                    let mut dict_encoder = DictionaryEncoder::new();
                    let mut data_buf = Vec::new();
                    for v in values {
                        if let Some(s) = v.as_str() {
                            let id = dict_encoder.encode(s);
                            data_buf.extend_from_slice(&id.to_be_bytes());
                        }
                    }
                    let (dict_data, _) = dict_encoder.finish();
                    string_fields.insert(field_name.clone(), data_buf);
                    dictionaries.insert(field_name.clone(), dict_data);
                }
                FieldValue::Boolean(_) => {
                    let mut buf = Vec::new();
                    for (i, v) in values.iter().enumerate() {
                        if let Some(b) = v.as_bool() {
                            let byte_idx = i / 8;
                            let bit_idx = (i % 8) as u8;
                            if byte_idx >= buf.len() {
                                buf.push(0u8);
                            }
                            if b {
                                buf[byte_idx] |= 1 << bit_idx;
                            }
                        }
                    }
                    int_fields.insert(field_name.clone(), buf);
                }
            }
        }

        Ok(CompressedBlock {
            timestamps: ts_encoder.finish(),
            float_fields,
            int_fields,
            string_fields,
            dictionaries,
            row_count: block.timestamps.len(),
        })
    }

    /// 将 CompressedBlock 解压还原为原始 DataBlock
    ///
    /// # 参数
    /// - `compressed`: 已压缩的数据块
    ///
    /// # 返回
    /// - `Ok(DataBlock)`: 还原后的原始数据块
    /// - `Err(CompressError)`: 解码过程中发生错误（如格式损坏）
    fn decompress_block(&self, compressed: &CompressedBlock) -> CompressResult<DataBlock> {
        let ts_decoder = DeltaDecoder::new(compressed.timestamps.clone());
        let timestamps = ts_decoder.decode_all()?;

        let mut fields = HashMap::new();

        for (field_name, data) in &compressed.float_fields {
            let decoder = GorillaDecoder::new(data.clone())?;
            let values = decoder.decode_all()?;
            fields.insert(
                field_name.clone(),
                values.into_iter().map(FieldValue::Float).collect(),
            );
        }

        for (field_name, data) in &compressed.int_fields {
            if !compressed.dictionaries.contains_key(field_name) {
                let chunk_size = 8;
                let mut values = Vec::new();
                for chunk in data.chunks(chunk_size) {
                    if chunk.len() == 8 {
                        let v = i64::from_be_bytes(
                            chunk
                                .try_into()
                                .map_err(|_| CompressError::Decode("invalid int".into()))?,
                        );
                        values.push(FieldValue::Integer(v));
                    }
                }
                fields.insert(field_name.clone(), values);
            }
        }

        for (field_name, data) in &compressed.string_fields {
            if let Some(dict_data) = compressed.dictionaries.get(field_name) {
                let (decoder, _) = DictionaryDecoder::from_encoded(dict_data)?;
                let mut values = Vec::new();
                for chunk in data.chunks(4) {
                    if chunk.len() == 4 {
                        let id = u32::from_be_bytes(
                            chunk
                                .try_into()
                                .map_err(|_| CompressError::Decode("invalid string id".into()))?,
                        );
                        if let Some(s) = decoder.decode(id) {
                            values.push(FieldValue::String(s.to_string()));
                        }
                    }
                }
                fields.insert(field_name.clone(), values);
            }
        }

        Ok(DataBlock { timestamps, fields })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_codec_roundtrip() {
        let block = DataBlock {
            timestamps: vec![1_000_000_000, 1_000_030_000, 1_000_060_000],
            fields: {
                let mut m = HashMap::new();
                m.insert(
                    "cpu".to_string(),
                    vec![
                        FieldValue::Float(0.5),
                        FieldValue::Float(0.6),
                        FieldValue::Float(0.7),
                    ],
                );
                m.insert(
                    "count".to_string(),
                    vec![
                        FieldValue::Integer(10),
                        FieldValue::Integer(20),
                        FieldValue::Integer(30),
                    ],
                );
                m
            },
        };

        let codec = BlockCodec;
        let compressed = codec.compress_block(&block).unwrap();
        let decompressed = codec.decompress_block(&compressed).unwrap();

        assert_eq!(decompressed.timestamps, block.timestamps);
        assert_eq!(decompressed.fields.len(), block.fields.len());
    }
}
