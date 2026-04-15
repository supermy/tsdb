use tsdb_types::model::{DataPoint, FieldValue};

pub trait BusinessPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn validate(&self, dp: &DataPoint) -> bool;
    fn default_aggregations(&self) -> Vec<String>;
}

pub trait QueryPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn supported_functions(&self) -> Vec<String>;
    fn evaluate(&self, func_name: &str, args: &[FieldValue]) -> Option<FieldValue>;
}

pub trait StoragePlugin: Send + Sync {
    fn name(&self) -> &str;
    fn write(&self, dp: &DataPoint) -> anyhow::Result<()>;
    fn read(&self, measurement: &str, start: i64, end: i64) -> anyhow::Result<Vec<DataPoint>>;
}
