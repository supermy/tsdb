//! RocksDB 属性模块 - User-Collected Properties
//!
//! 自定义 RocksDB 属性收集器，用于在 SST 文件中存储 TSDB 特定的统计信息。
//! 可通过 DB::property_value() 查询这些属性。

use std::collections::HashMap;

pub struct TsdbProperties {
    properties: HashMap<String, String>,
}

impl TsdbProperties {
    pub fn new() -> Self {
        let mut properties = HashMap::new();
        properties.insert("tsdb.series_count".to_string(), "0".to_string());
        properties.insert("tsdb.data_point_count".to_string(), "0".to_string());
        properties.insert("tsdb.time_range_start".to_string(), i64::MAX.to_string());
        properties.insert("tsdb.time_range_end".to_string(), i64::MIN.to_string());
        properties.insert("tsdb.compression_ratio".to_string(), "0.0".to_string());
        Self { properties }
    }

    pub fn set(&mut self, key: &str, value: &str) {
        self.properties.insert(key.to_string(), value.to_string());
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.properties.get(key).map(|s| s.as_str())
    }

    pub fn increment(&mut self, key: &str, delta: u64) {
        let current: u64 = self.properties.get(key)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        self.properties.insert(key.to_string(), (current + delta).to_string());
    }

    pub fn update_time_range(&mut self, timestamp: i64) {
        let current_start: i64 = self.properties.get("tsdb.time_range_start")
            .and_then(|v| v.parse().ok())
            .unwrap_or(i64::MAX);
        let current_end: i64 = self.properties.get("tsdb.time_range_end")
            .and_then(|v| v.parse().ok())
            .unwrap_or(i64::MIN);

        if timestamp < current_start {
            self.properties.insert("tsdb.time_range_start".to_string(), timestamp.to_string());
        }
        if timestamp > current_end {
            self.properties.insert("tsdb.time_range_end".to_string(), timestamp.to_string());
        }
    }

    pub fn all_properties(&self) -> &HashMap<String, String> {
        &self.properties
    }

    pub fn to_int_property(&self, key: &str) -> u64 {
        self.properties.get(key)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }
}

impl Default for TsdbProperties {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_properties_basic() {
        let mut props = TsdbProperties::new();
        props.set("tsdb.series_count", "42");
        assert_eq!(props.get("tsdb.series_count"), Some("42"));
    }

    #[test]
    fn test_properties_increment() {
        let mut props = TsdbProperties::new();
        props.increment("tsdb.data_point_count", 100);
        props.increment("tsdb.data_point_count", 50);
        assert_eq!(props.to_int_property("tsdb.data_point_count"), 150);
    }

    #[test]
    fn test_properties_time_range() {
        let mut props = TsdbProperties::new();
        props.update_time_range(1000);
        props.update_time_range(500);
        props.update_time_range(2000);
        assert_eq!(props.get("tsdb.time_range_start"), Some("500"));
        assert_eq!(props.get("tsdb.time_range_end"), Some("2000"));
    }
}
