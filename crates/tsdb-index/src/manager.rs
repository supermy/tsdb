use crate::skiplist::SkipList;
use crate::inverted::InvertedIndex;
use tsdb_types::model::SeriesId;
use std::collections::HashMap;

pub struct IndexManager {
    time_index: HashMap<String, SkipList>,
    tag_index: HashMap<String, InvertedIndex>,
    next_series_id: SeriesId,
    series_cache: HashMap<String, SeriesId>,
}

impl IndexManager {
    pub fn new() -> Self {
        Self {
            time_index: HashMap::new(),
            tag_index: HashMap::new(),
            next_series_id: 1,
            series_cache: HashMap::new(),
        }
    }

    pub fn index_data_point(
        &mut self,
        measurement: &str,
        tags: &std::collections::BTreeMap<String, String>,
        timestamp: i64,
        block_offset: u64,
    ) -> SeriesId {
        let series_key = format!("{},{}", measurement,
            tags.iter().map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<_>>().join(","));

        let series_id = *self.series_cache
            .entry(series_key.clone())
            .or_insert_with(|| {
                let id = self.next_series_id;
                self.next_series_id += 1;

                let tag_index = self.tag_index
                    .entry(measurement.to_string())
                    .or_insert_with(InvertedIndex::new);
                let tag_pairs: Vec<(String, String)> = tags.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                tag_index.add_series(id, &tag_pairs);

                id
            });

        let time_index = self.time_index
            .entry(measurement.to_string())
            .or_insert_with(|| SkipList::new(16));
        time_index.insert(timestamp, block_offset);

        series_id
    }

    pub fn query_by_time_range(
        &self,
        measurement: &str,
        start: i64,
        end: i64,
    ) -> Vec<(i64, Vec<u64>)> {
        if let Some(time_idx) = self.time_index.get(measurement) {
            time_idx.range_query(start, end)
        } else {
            Vec::new()
        }
    }

    pub fn query_by_tags(
        &self,
        measurement: &str,
        filters: &[(String, String)],
    ) -> roaring::RoaringBitmap {
        if let Some(tag_idx) = self.tag_index.get(measurement) {
            tag_idx.query_intersection(filters)
        } else {
            roaring::RoaringBitmap::new()
        }
    }

    pub fn get_series_id(&self, series_key: &str) -> Option<SeriesId> {
        self.series_cache.get(series_key).copied()
    }

    pub fn series_count(&self, measurement: &str) -> usize {
        self.tag_index.get(measurement)
            .map(|idx| idx.series_count())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_data_point() {
        let mut mgr = IndexManager::new();
        let mut tags = std::collections::BTreeMap::new();
        tags.insert("host".to_string(), "server01".to_string());

        let sid = mgr.index_data_point("cpu", &tags, 1_000_000_000, 0);
        assert_eq!(sid, 1);

        let sid2 = mgr.index_data_point("cpu", &tags, 1_000_030_000, 1);
        assert_eq!(sid2, 1);

        let results = mgr.query_by_time_range("cpu", 0, 2_000_000_000);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_tag_query() {
        let mut mgr = IndexManager::new();
        let mut tags1 = std::collections::BTreeMap::new();
        tags1.insert("host".to_string(), "server01".to_string());
        tags1.insert("region".to_string(), "us-west".to_string());

        let mut tags2 = std::collections::BTreeMap::new();
        tags2.insert("host".to_string(), "server02".to_string());
        tags2.insert("region".to_string(), "us-west".to_string());

        mgr.index_data_point("cpu", &tags1, 1_000_000_000, 0);
        mgr.index_data_point("cpu", &tags2, 1_000_000_000, 1);

        let result = mgr.query_by_tags("cpu", &[
            ("region".to_string(), "us-west".to_string()),
        ]);
        assert_eq!(result.len(), 2);

        let result = mgr.query_by_tags("cpu", &[
            ("host".to_string(), "server01".to_string()),
        ]);
        assert_eq!(result.len(), 1);
    }
}
