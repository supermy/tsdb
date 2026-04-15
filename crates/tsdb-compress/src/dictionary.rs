use std::collections::HashMap;

pub struct DictionaryEncoder {
    dictionary: HashMap<String, u32>,
    next_id: u32,
    encoded: Vec<u8>,
}

impl DictionaryEncoder {
    pub fn new() -> Self {
        Self {
            dictionary: HashMap::new(),
            next_id: 0,
            encoded: Vec::new(),
        }
    }

    pub fn encode(&mut self, value: &str) -> u32 {
        if let Some(&id) = self.dictionary.get(value) {
            return id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.dictionary.insert(value.to_string(), id);

        let value_bytes = value.as_bytes();
        self.encoded.push(1u8);
        self.encoded.extend_from_slice(&(value_bytes.len() as u16).to_be_bytes());
        self.encoded.extend_from_slice(value_bytes);
        self.encoded.extend_from_slice(&id.to_be_bytes());

        id
    }

    pub fn finish(self) -> (Vec<u8>, HashMap<String, u32>) {
        (self.encoded, self.dictionary)
    }
}

pub struct DictionaryDecoder {
    dictionary: HashMap<u32, String>,
}

impl DictionaryDecoder {
    pub fn new(dictionary: HashMap<u32, String>) -> Self {
        Self { dictionary }
    }

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

            let id = u32::from_be_bytes(data[pos..pos + 4].try_into().map_err(|_| CompressError::Decode("invalid id".into()))?);
            pos += 4;

            dictionary.insert(id, value);
        }

        Ok((Self { dictionary }, pos))
    }

    pub fn decode(&self, id: u32) -> Option<&str> {
        self.dictionary.get(&id).map(|s| s.as_str())
    }

    pub fn dictionary(&self) -> &HashMap<u32, String> {
        &self.dictionary
    }
}

use crate::error::{CompressError, CompressResult};

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
        assert_eq!(consumed, encoded.len());
    }
}
