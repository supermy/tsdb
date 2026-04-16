//! # 字典编码（Dictionary Encoding）— 字符串压缩
//!
//! ## 设计动机
//!
//! 时间序列数据中的字符串字段（如 measurement 名称、tag value）通常
//! 在同一批次中大量重复。字典编码通过维护一个 **字符串→ID** 映射表，
//! 将重复的字符串替换为紧凑的整型 ID，显著减少存储空间。
//!
//! ## 编码格式
//!
//! ```text
//! 字典条目: [0x01] [长度:u16] [字符串字节] [ID:u32]
//! 数据部分: [ID:u32] [ID:u32] ...
//!
//! 示例:
//!   编码 "cpu" → ID=0, "mem" → ID=1, "cpu" → ID=0 (复用)
//!   原始: ["cpu", "mem", "cpu"]  (11 字节)
//!   编码: [dict_entries] + [0, 1, 0]  (12 字节 + dict)
//!   大量重复时效果更显著
//! ```
//!

use std::collections::HashMap;

use crate::error::{CompressError, CompressResult};

/// 字典编码器 — 将字符串序列映射为整型 ID 并生成字典数据
///
/// 维护一个增量构建的 `String → u32` 映射表：
/// - 首次遇到的字符串：分配新 ID，写入字典条目到输出流
/// - 已存在的字符串：直接返回已有 ID（零额外空间开销）
///
/// ## 输出结构
///
/// 调用 `finish()` 后返回两部分数据：
/// 1. **encoded**: 字典条目 + ID 序列的二进制数据
/// 2. **dictionary**: 完整的映射表（用于构造 Decoder）
#[derive(Default)]
pub struct DictionaryEncoder {
    dictionary: HashMap<String, u32>,
    next_id: u32,
    encoded: Vec<u8>,
}

impl DictionaryEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 对单个字符串进行字典编码
    ///
    /// 如果该字符串已在字典中则直接返回已有 ID；
    /// 否则分配新 ID 并将字典条目追加到 encoded 输出中。
    ///
    /// ## 字典条目格式
    ///
    /// ```text
    /// [0x01 标记] [字符串长度: u16 Big-Endian] [字符串 UTF-8 字节] [ID: u32 Big-Endian]
    /// ```
    ///
    /// # 参数
    /// - `value`: 待编码的字符串
    ///
    /// # 返回
    /// 该字符串对应的唯一 ID（u32）
    pub fn encode(&mut self, value: &str) -> u32 {
        if let Some(&id) = self.dictionary.get(value) {
            return id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.dictionary.insert(value.to_string(), id);

        let value_bytes = value.as_bytes();
        self.encoded.push(1u8);
        self.encoded
            .extend_from_slice(&(value_bytes.len() as u16).to_be_bytes());
        self.encoded.extend_from_slice(value_bytes);
        self.encoded.extend_from_slice(&id.to_be_bytes());

        id
    }

    /// 结束编码并返回结果
    ///
    /// # 返回
    /// 元组 `(encoded_data, dictionary_map)`:
    /// - `encoded_data`: 可持久化/传输的二进制字典数据
    /// - `dictionary_map`: String→ID 映射表（用于反向查找）
    pub fn finish(self) -> (Vec<u8>, HashMap<String, u32>) {
        (self.encoded, self.dictionary)
    }
}

/// 字典解码器 — 将整型 ID 还原为原始字符串
///
/// 使用预加载的 `u32 → String` 反向映射表进行 O(1) 查找解码。
pub struct DictionaryDecoder {
    /// ID → 字符串的反向映射表
    dictionary: HashMap<u32, String>,
}

impl DictionaryDecoder {
    /// 从预先构建的反向映射表创建解码器
    ///
    /// # 参数
    /// - `dictionary`: ID → String 的映射（通常由 Encoder.finish() 结果反转得到）
    pub fn new(dictionary: HashMap<u32, String>) -> Self {
        Self { dictionary }
    }

    /// 从编码后的二进制数据自动重建字典并创建解码器
    ///
    /// 解析 DictionaryEncoder 生成的二进制格式，
    /// 逐个提取字典条目以重建完整的 ID→String 映射。
    ///
    /// # 参数
    /// - `data`: DictionaryEncoder.finish() 返回的 encoded 数据
    ///
    /// # 返回
    /// 元组 `(decoder, consumed_bytes)`:
    /// - `decoder`: 已初始化的解码器实例
    /// - `consumed_bytes`: 从 data 中消耗的字节数（字典部分长度）
    pub fn from_encoded(data: &[u8]) -> CompressResult<(Self, usize)> {
        let mut dictionary = HashMap::new();
        let mut pos = 0;

        while pos < data.len() {
            if data[pos] != 1 {
                break;
            }
            pos += 1;

            if pos + 2 > data.len() {
                return Err(CompressError::Decode("invalid dictionary data".into()));
            }
            let len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;

            if pos + len + 4 > data.len() {
                return Err(CompressError::Decode("invalid dictionary data".into()));
            }
            let value = String::from_utf8_lossy(&data[pos..pos + len]).to_string();
            pos += len;

            let id = u32::from_be_bytes(
                data[pos..pos + 4]
                    .try_into()
                    .map_err(|_| CompressError::Decode("invalid id".into()))?,
            );
            pos += 4;

            dictionary.insert(id, value);
        }

        Ok((Self { dictionary }, pos))
    }

    /// 根据 ID 反查原始字符串
    ///
    /// # 参数
    /// - `id`: 通过 DictionaryEncoder.encode() 获得的 ID
    ///
    /// # 返回
    /// - `Some(&str)`: 对应的原始字符串引用
    /// - `None`: 该 ID 不在字典中
    pub fn decode(&self, id: u32) -> Option<&str> {
        self.dictionary.get(&id).map(|s| s.as_str())
    }

    /// 获取内部字典的不可变引用（用于调试或导出）
    pub fn dictionary(&self) -> &HashMap<u32, String> {
        &self.dictionary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dictionary_encode_decode() {
        let mut encoder = DictionaryEncoder::new();
        let id1 = encoder.encode("hello");
        let id2 = encoder.encode("world");
        let id3 = encoder.encode("hello");

        assert_eq!(id1, id3);
        assert_ne!(id1, id2);

        let (encoded, dict) = encoder.finish();
        let _ = encoded;

        let mut reverse_dict: HashMap<u32, String> = HashMap::new();
        for (k, v) in dict {
            reverse_dict.insert(v, k);
        }

        let decoder = DictionaryDecoder::new(reverse_dict);
        assert_eq!(decoder.decode(id1), Some("hello"));
        assert_eq!(decoder.decode(id2), Some("world"));
    }

    #[test]
    fn test_dictionary_from_encoded() {
        let mut encoder = DictionaryEncoder::new();
        encoder.encode("cpu");
        encoder.encode("memory");
        encoder.encode("disk");

        let (encoded, _) = encoder.finish();

        let (decoder, consumed) = DictionaryDecoder::from_encoded(&encoded).unwrap();
        let _ = decoder;
        assert_eq!(consumed, encoded.len());
    }
}
