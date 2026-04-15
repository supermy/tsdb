use crate::error::{Result, TsdbError};
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const DIMENSION_CF: &str = "dimension";

pub struct DimensionTable {
    db: Arc<DB>,
    tag_key_ids: std::sync::Mutex<HashMap<String, u32>>,
    tag_value_ids: std::sync::Mutex<HashMap<(u32, String), u32>>,
    next_key_id: AtomicU64,
    next_value_id: AtomicU64,
}

impl DimensionTable {
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            db,
            tag_key_ids: std::sync::Mutex::new(HashMap::new()),
            tag_value_ids: std::sync::Mutex::new(HashMap::new()),
            next_key_id: AtomicU64::new(1),
            next_value_id: AtomicU64::new(1),
        }
    }

    pub fn encode_tag_key(&self, key: &str) -> u32 {
        let mut map = self.tag_key_ids.lock().unwrap();
        if let Some(&id) = map.get(key) {
            return id;
        }
        let id = self.next_key_id.fetch_add(1, Ordering::Relaxed) as u32;
        map.insert(key.to_string(), id);
        id
    }

    pub fn decode_tag_key(&self, id: u32) -> Option<String> {
        let map = self.tag_key_ids.lock().unwrap();
        map.iter().find(|(_, &v)| v == id).map(|(k, _)| k.clone())
    }

    pub fn encode_tag_value(&self, key_id: u32, value: &str) -> u32 {
        let mut map = self.tag_value_ids.lock().unwrap();
        let key = (key_id, value.to_string());
        if let Some(&id) = map.get(&key) {
            return id;
        }
        let id = self.next_value_id.fetch_add(1, Ordering::Relaxed) as u32;
        map.insert(key, id);
        id
    }

    pub fn decode_tag_value(&self, key_id: u32, value_id: u32) -> Option<String> {
        let map = self.tag_value_ids.lock().unwrap();
        map.iter()
            .find(|((k, _), &v)| *k == key_id && v == value_id)
            .map(|((_, v), _)| v.clone())
    }

    pub fn encode_tags(&self, tags: &tsdb_types::model::Tags) -> Vec<(u32, u32)> {
        let mut result = Vec::with_capacity(tags.len());
        for (key, value) in tags {
            let key_id = self.encode_tag_key(key);
            let value_id = self.encode_tag_value(key_id, value);
            result.push((key_id, value_id));
        }
        result.sort_by_key(|(k, _)| *k);
        result
    }

    pub fn decode_tags(&self, encoded: &[(u32, u32)]) -> tsdb_types::model::Tags {
        let mut tags = tsdb_types::model::Tags::new();
        for (key_id, value_id) in encoded {
            if let Some(key) = self.decode_tag_key(*key_id) {
                if let Some(value) = self.decode_tag_value(*key_id, *value_id) {
                    tags.insert(key, value);
                }
            }
        }
        tags
    }

    pub fn compute_tag_signature(&self, tags: &tsdb_types::model::Tags) -> u64 {
        let encoded = self.encode_tags(tags);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash, Hasher};
        for (k, v) in &encoded {
            k.hash(&mut hasher);
            v.hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn tag_key_count(&self) -> usize {
        self.tag_key_ids.lock().unwrap().len()
    }

    pub fn tag_value_count(&self) -> usize {
        self.tag_value_ids.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_tag_key() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = Arc::new(DB::open_default(dir.path()).unwrap());
        let dim = DimensionTable::new(db);

        let id1 = dim.encode_tag_key("host");
        let id2 = dim.encode_tag_key("region");
        let id3 = dim.encode_tag_key("host");

        assert_eq!(id1, id3);
        assert_ne!(id1, id2);

        assert_eq!(dim.decode_tag_key(id1), Some("host".to_string()));
        assert_eq!(dim.decode_tag_key(id2), Some("region".to_string()));
    }

    #[test]
    fn test_encode_decode_tags() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = Arc::new(DB::open_default(dir.path()).unwrap());
        let dim = DimensionTable::new(db);

        let mut tags = tsdb_types::model::Tags::new();
        tags.insert("host".to_string(), "server01".to_string());
        tags.insert("region".to_string(), "us-west".to_string());

        let encoded = dim.encode_tags(&tags);
        let decoded = dim.decode_tags(&encoded);

        assert_eq!(decoded, tags);
    }

    #[test]
    fn test_tag_signature() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = Arc::new(DB::open_default(dir.path()).unwrap());
        let dim = DimensionTable::new(db);

        let mut tags1 = tsdb_types::model::Tags::new();
        tags1.insert("host".to_string(), "server01".to_string());

        let mut tags2 = tsdb_types::model::Tags::new();
        tags2.insert("host".to_string(), "server01".to_string());

        assert_eq!(dim.compute_tag_signature(&tags1), dim.compute_tag_signature(&tags2));
    }
}
