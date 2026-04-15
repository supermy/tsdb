use crate::delta::{DeltaEncoder, DeltaDecoder};
use crate::gorilla::{GorillaEncoder, GorillaDecoder};
use crate::dictionary::{DictionaryEncoder, DictionaryDecoder};
use crate::error::{CompressError, CompressResult};
use tsdb_types::model::FieldValue;
use std::collections::HashMap;

pub trait Codec {
    fn compress_block(&self, block: &DataBlock) -> CompressResult<CompressedBlock>;
    fn decompress_block(&self, compressed: &CompressedBlock) -> CompressResult<DataBlock>;
}

#[derive(Debug, Clone)]
pub struct DataBlock {
    pub timestamps: Vec<i64>,
    pub fields: HashMap<String, Vec<FieldValue>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompressedBlock {
    pub timestamps: Vec<u8>,
    pub float_fields: HashMap<String, Vec<u8>>,
    pub int_fields: HashMap<String, Vec<u8>>,
    pub string_fields: HashMap<String, Vec<u8>>,
    pub dictionaries: HashMap<String, Vec<u8>>,
    pub row_count: usize,
}

pub struct BlockCodec;

impl Codec for BlockCodec {
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
                        let v = i64::from_be_bytes(chunk.try_into().map_err(|_| CompressError::Decode("invalid int".into()))?);
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
                        let id = u32::from_be_bytes(chunk.try_into().map_err(|_| CompressError::Decode("invalid string id".into()))?);
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
                m.insert("cpu".to_string(), vec![
                    FieldValue::Float(0.5),
                    FieldValue::Float(0.6),
                    FieldValue::Float(0.7),
                ]);
                m.insert("count".to_string(), vec![
                    FieldValue::Integer(10),
                    FieldValue::Integer(20),
                    FieldValue::Integer(30),
                ]);
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
