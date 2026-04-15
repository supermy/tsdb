use tsdb_types::model::{DataPoint, FieldValue, Timestamp};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeDimension {
    Hour,
    Day,
    Week,
    Month,
}

impl TimeDimension {
    pub fn micros(&self) -> i64 {
        match self {
            TimeDimension::Hour => 3_600_000_000,
            TimeDimension::Day => 86_400_000_000,
            TimeDimension::Week => 604_800_000_000,
            TimeDimension::Month => 2_592_000_000_000,
        }
    }

    pub fn align(&self, ts: Timestamp) -> Timestamp {
        match self {
            TimeDimension::Hour => (ts / 3_600_000_000) * 3_600_000_000,
            TimeDimension::Day => (ts / 86_400_000_000) * 86_400_000_000,
            TimeDimension::Week => (ts / 604_800_000_000) * 604_800_000_000,
            TimeDimension::Month => (ts / 2_592_000_000_000) * 2_592_000_000_000,
        }
    }

    pub fn cf_suffix(&self) -> &str {
        match self {
            TimeDimension::Hour => "hour",
            TimeDimension::Day => "day",
            TimeDimension::Week => "week",
            TimeDimension::Month => "month",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AggFunc {
    Sum,
    Avg,
    Min,
    Max,
    Count,
    First,
    Last,
}

#[derive(Debug, Clone)]
pub struct AggregateSpec {
    pub time_dimension: TimeDimension,
    pub field_name: String,
    pub func: AggFunc,
}

#[derive(Debug, Clone)]
pub struct AggregateResult {
    pub time_bucket: Timestamp,
    pub measurement: String,
    pub tag_hash: u64,
    pub field_name: String,
    pub func: AggFunc,
    pub value: FieldValue,
}

pub struct Aggregator;

impl Aggregator {
    pub fn aggregate(
        data_points: &[DataPoint],
        spec: &AggregateSpec,
    ) -> Vec<AggregateResult> {
        let mut buckets: HashMap<(Timestamp, String, u64), Vec<&FieldValue>> = HashMap::new();

        for dp in data_points {
            if let Some(fv) = dp.fields.get(&spec.field_name) {
                let bucket_ts = spec.time_dimension.align(dp.timestamp);
                let tag_hash = {
                    use std::hash::{Hasher, Hash};
                    let mut hasher = std::hash::DefaultHasher::new();
                    for (k, v) in &dp.tags {
                        k.hash(&mut hasher);
                        v.hash(&mut hasher);
                    }
                    hasher.finish()
                };
                let key = (bucket_ts, dp.measurement.clone(), tag_hash);
                buckets.entry(key).or_default().push(fv);
            }
        }

        let mut results = Vec::new();
        for ((bucket_ts, measurement, tag_hash), values) in buckets {
            let value = Self::compute(spec.func, &values);
            results.push(AggregateResult {
                time_bucket: bucket_ts,
                measurement,
                tag_hash,
                field_name: spec.field_name.clone(),
                func: spec.func,
                value,
            });
        }

        results.sort_by_key(|r| r.time_bucket);
        results
    }

    fn compute(func: AggFunc, values: &[&FieldValue]) -> FieldValue {
        match func {
            AggFunc::Count => FieldValue::Integer(values.len() as i64),
            AggFunc::Sum => {
                let sum: f64 = values.iter().filter_map(|v| v.as_f64()).sum();
                FieldValue::Float(sum)
            }
            AggFunc::Avg => {
                let nums: Vec<f64> = values.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    FieldValue::Float(0.0)
                } else {
                    FieldValue::Float(nums.iter().sum::<f64>() / nums.len() as f64)
                }
            }
            AggFunc::Min => {
                FieldValue::Float(values.iter()
                    .filter_map(|v| v.as_f64())
                    .fold(f64::INFINITY, f64::min))
            }
            AggFunc::Max => {
                FieldValue::Float(values.iter()
                    .filter_map(|v| v.as_f64())
                    .fold(f64::NEG_INFINITY, f64::max))
            }
            AggFunc::First => values.first().cloned().cloned().unwrap_or(FieldValue::Integer(0)),
            AggFunc::Last => values.last().cloned().cloned().unwrap_or(FieldValue::Integer(0)),
        }
    }

    pub fn encode_aggregate_key(
        measurement: &str,
        tag_hash: u64,
        time_bucket: Timestamp,
        field_name: &str,
        func: AggFunc,
    ) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(measurement.as_bytes());
        buf.push(b'|');
        buf.extend_from_slice(&tag_hash.to_be_bytes());
        buf.push(b'|');
        buf.extend_from_slice(&time_bucket.to_be_bytes());
        buf.push(b'|');
        buf.extend_from_slice(field_name.as_bytes());
        buf.push(b':');
        buf.push(func as u8);
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsdb_types::model::{DataPoint, FieldValue};

    fn make_dp(measurement: &str, ts: Timestamp, field_name: &str, value: f64) -> DataPoint {
        let mut dp = DataPoint::new(measurement, ts);
        dp.fields.insert(field_name.to_string(), FieldValue::Float(value));
        dp
    }

    #[test]
    fn test_hourly_aggregation() {
        let data_points: Vec<DataPoint> = vec![
            make_dp("cpu", 1_000_000_000, "usage", 0.5),
            make_dp("cpu", 1_800_000_000, "usage", 0.7),
            make_dp("cpu", 3_600_000_000, "usage", 0.9),
        ];

        let spec = AggregateSpec {
            time_dimension: TimeDimension::Hour,
            field_name: "usage".to_string(),
            func: AggFunc::Avg,
        };

        let results = Aggregator::aggregate(&data_points, &spec);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_time_alignment() {
        assert_eq!(TimeDimension::Hour.align(1_800_000_000), 0);
        assert_eq!(TimeDimension::Hour.align(3_600_000_000), 3_600_000_000);
        assert_eq!(TimeDimension::Day.align(86_400_000_000), 86_400_000_000);
    }
}
