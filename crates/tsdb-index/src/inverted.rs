use roaring::RoaringBitmap;
use std::collections::HashMap;
use tsdb_types::model::SeriesId;

pub struct InvertedIndex {
    postings: HashMap<String, RoaringBitmap>,
    series_tags: HashMap<SeriesId, Vec<(String, String)>>,
}

impl InvertedIndex {
    pub fn new() -> Self {
        Self {
            postings: HashMap::new(),
            series_tags: HashMap::new(),
        }
    }

    pub fn add_series(&mut self, series_id: SeriesId, tags: &[(String, String)]) {
        self.series_tags.insert(series_id, tags.to_vec());
        for (key, value) in tags {
            let posting_key = format!("{}={}", key, value);
            self.postings
                .entry(posting_key)
                .or_insert_with(RoaringBitmap::new)
                .insert(series_id as u32);
        }
    }

    pub fn remove_series(&mut self, series_id: SeriesId) {
        if let Some(tags) = self.series_tags.remove(&series_id) {
            for (key, value) in tags {
                let posting_key = format!("{}={}", key, value);
                if let Some(bitmap) = self.postings.get_mut(&posting_key) {
                    bitmap.remove(series_id as u32);
                    if bitmap.is_empty() {
                        self.postings.remove(&posting_key);
                    }
                }
            }
        }
    }

    pub fn query_exact(&self, tag_key: &str, tag_value: &str) -> RoaringBitmap {
        let posting_key = format!("{}={}", tag_key, tag_value);
        self.postings.get(&posting_key).cloned().unwrap_or_default()
    }

    pub fn query_intersection(&self, filters: &[(String, String)]) -> RoaringBitmap {
        if filters.is_empty() {
            return RoaringBitmap::new();
        }

        let bitmaps: Vec<RoaringBitmap> = filters
            .iter()
            .map(|(k, v)| self.query_exact(k, v))
            .collect();

        let mut result = bitmaps[0].clone();
        for bitmap in &bitmaps[1..] {
            result &= bitmap;
        }
        result
    }

    pub fn query_union(&self, filters: &[(String, String)]) -> RoaringBitmap {
        let mut result = RoaringBitmap::new();
        for (k, v) in filters {
            let posting_key = format!("{}={}", k, v);
            if let Some(bitmap) = self.postings.get(&posting_key) {
                result |= bitmap;
            }
        }
        result
    }

    pub fn series_count(&self) -> usize {
        self.series_tags.len()
    }

    pub fn posting_count(&self) -> usize {
        self.postings.len()
    }

    pub fn get_series_tags(&self, series_id: SeriesId) -> Option<&[(String, String)]> {
        self.series_tags.get(&series_id).map(|v| v.as_slice())
    }

    pub fn all_series_ids(&self) -> RoaringBitmap {
        let mut bitmap = RoaringBitmap::new();
        for &id in self.series_tags.keys() {
            bitmap.insert(id as u32);
        }
        bitmap
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(self.postings.len() as u32).to_le_bytes());
        for (key, bitmap) in &self.postings {
            let key_bytes = key.as_bytes();
            buf.extend_from_slice(&(key_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(key_bytes);
            let mut bm_data = Vec::new();
            bitmap.serialize_into(&mut bm_data).unwrap_or(());
            buf.extend_from_slice(&(bm_data.len() as u32).to_le_bytes());
            buf.extend_from_slice(&bm_data);
        }

        buf.extend_from_slice(&(self.series_tags.len() as u32).to_le_bytes());
        for (&id, tags) in &self.series_tags {
            buf.extend_from_slice(&id.to_le_bytes());
            buf.extend_from_slice(&(tags.len() as u32).to_le_bytes());
            for (k, v) in tags {
                buf.extend_from_slice(&(k.len() as u32).to_le_bytes());
                buf.extend_from_slice(k.as_bytes());
                buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
                buf.extend_from_slice(v.as_bytes());
            }
        }
        buf
    }

    pub fn deserialize(data: &[u8]) -> Option<Self> {
        let mut offset = 0;
        let postings_count = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        offset += 4;

        let mut postings = HashMap::new();
        for _ in 0..postings_count {
            let key_len = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
            offset += 4;
            let key = String::from_utf8_lossy(&data[offset..offset + key_len]).to_string();
            offset += key_len;
            let bm_len = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
            offset += 4;
            let bitmap = RoaringBitmap::deserialize_from(&data[offset..offset + bm_len]).ok()?;
            offset += bm_len;
            postings.insert(key, bitmap);
        }

        let series_count = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        offset += 4;

        let mut series_tags = HashMap::new();
        for _ in 0..series_count {
            let id = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            offset += 8;
            let tags_count = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
            offset += 4;
            let mut tags = Vec::with_capacity(tags_count);
            for _ in 0..tags_count {
                let k_len = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
                offset += 4;
                let k = String::from_utf8_lossy(&data[offset..offset + k_len]).to_string();
                offset += k_len;
                let v_len = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
                offset += 4;
                let v = String::from_utf8_lossy(&data[offset..offset + v_len]).to_string();
                offset += v_len;
                tags.push((k, v));
            }
            series_tags.insert(id, tags);
        }

        Some(Self { postings, series_tags })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_query() {
        let mut idx = InvertedIndex::new();
        idx.add_series(1, &[
            ("host".to_string(), "server01".to_string()),
            ("region".to_string(), "us-west".to_string()),
        ]);
        idx.add_series(2, &[
            ("host".to_string(), "server02".to_string()),
            ("region".to_string(), "us-west".to_string()),
        ]);
        idx.add_series(3, &[
            ("host".to_string(), "server01".to_string()),
            ("region".to_string(), "eu-central".to_string()),
        ]);

        let result = idx.query_exact("host", "server01");
        assert!(result.contains(1));
        assert!(!result.contains(2));
        assert!(result.contains(3));
    }

    #[test]
    fn test_intersection() {
        let mut idx = InvertedIndex::new();
        idx.add_series(1, &[
            ("host".to_string(), "server01".to_string()),
            ("region".to_string(), "us-west".to_string()),
        ]);
        idx.add_series(2, &[
            ("host".to_string(), "server01".to_string()),
            ("region".to_string(), "eu-central".to_string()),
        ]);

        let result = idx.query_intersection(&[
            ("host".to_string(), "server01".to_string()),
            ("region".to_string(), "us-west".to_string()),
        ]);

        assert!(result.contains(1));
        assert!(!result.contains(2));
    }

    #[test]
    fn test_remove_series() {
        let mut idx = InvertedIndex::new();
        idx.add_series(1, &[("host".to_string(), "server01".to_string())]);
        idx.remove_series(1);

        let result = idx.query_exact("host", "server01");
        assert!(result.is_empty());
    }
}
