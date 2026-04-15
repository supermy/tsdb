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
