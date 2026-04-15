use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type SeriesId = u64;
pub type Timestamp = i64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FieldValue {
    Float(f64),
    Integer(i64),
    String(String),
    Boolean(bool),
}

impl FieldValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            FieldValue::Float(v) => Some(*v),
            FieldValue::Integer(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            FieldValue::Integer(v) => Some(*v),
            FieldValue::Float(v) => Some(*v as i64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            FieldValue::String(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            FieldValue::Boolean(v) => Some(*v),
            _ => None,
        }
    }
}

pub type Tags = BTreeMap<String, String>;
pub type Fields = BTreeMap<String, FieldValue>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataPoint {
    pub measurement: String,
    pub tags: Tags,
    pub fields: Fields,
    pub timestamp: Timestamp,
}

impl DataPoint {
    pub fn new(measurement: impl Into<String>, timestamp: Timestamp) -> Self {
        Self {
            measurement: measurement.into(),
            tags: Tags::new(),
            fields: Fields::new(),
            timestamp,
        }
    }

    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }

    pub fn with_field(mut self, key: impl Into<String>, value: FieldValue) -> Self {
        self.fields.insert(key.into(), value);
        self
    }

    pub fn series_key(&self) -> String {
        let mut parts: Vec<String> = self.tags.iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        parts.sort();
        format!("{},{}", self.measurement, parts.join(","))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measurement {
    pub name: String,
    pub tag_keys: Vec<String>,
    pub field_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    Float,
    Integer,
    String,
    Boolean,
}

impl From<&FieldValue> for FieldType {
    fn from(value: &FieldValue) -> Self {
        match value {
            FieldValue::Float(_) => FieldType::Float,
            FieldValue::Integer(_) => FieldType::Integer,
            FieldValue::String(_) => FieldType::String,
            FieldValue::Boolean(_) => FieldType::Boolean,
        }
    }
}
