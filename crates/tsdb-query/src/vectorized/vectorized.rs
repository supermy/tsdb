use crate::vectorized::columnar::ColumnarBatch;
use crate::vectorized::simd_agg::{SimdAggFunc, SimdAggregator};
use std::collections::HashMap;
use tsdb_types::model::FieldValue;

pub struct VectorizedEngine;

impl VectorizedEngine {
    pub fn execute_aggregate(
        batch: &ColumnarBatch,
        column_name: &str,
        func: SimdAggFunc,
    ) -> Option<FieldValue> {
        SimdAggregator::aggregate(batch, column_name, func).map(FieldValue::Float)
    }

    pub fn execute_group_aggregate(
        batch: &ColumnarBatch,
        group_column: &str,
        agg_column: &str,
        func: SimdAggFunc,
    ) -> Vec<(String, FieldValue)> {
        SimdAggregator::group_aggregate(batch, group_column, agg_column, func)
            .into_iter()
            .map(|(k, v)| (k, FieldValue::Float(v)))
            .collect()
    }

    pub fn execute_multi_aggregate(
        batch: &ColumnarBatch,
        columns: &[(&str, SimdAggFunc)],
    ) -> HashMap<String, FieldValue> {
        let mut results = HashMap::new();
        for (col, func) in columns {
            let key = format!("{:?}({})", func, col);
            if let Some(v) = Self::execute_aggregate(batch, col, *func) {
                results.insert(key, v);
            }
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsdb_types::model::DataPoint;

    fn make_batch() -> ColumnarBatch {
        let dps = vec![
            {
                let mut dp = DataPoint::new("cpu", 1000);
                dp.tags.insert("host".to_string(), "s1".to_string());
                dp.fields
                    .insert("usage".to_string(), FieldValue::Float(0.5));
                dp.fields.insert("idle".to_string(), FieldValue::Float(0.5));
                dp
            },
            {
                let mut dp = DataPoint::new("cpu", 2000);
                dp.tags.insert("host".to_string(), "s1".to_string());
                dp.fields
                    .insert("usage".to_string(), FieldValue::Float(0.7));
                dp.fields.insert("idle".to_string(), FieldValue::Float(0.3));
                dp
            },
            {
                let mut dp = DataPoint::new("cpu", 3000);
                dp.tags.insert("host".to_string(), "s2".to_string());
                dp.fields
                    .insert("usage".to_string(), FieldValue::Float(0.9));
                dp.fields.insert("idle".to_string(), FieldValue::Float(0.1));
                dp
            },
        ];
        ColumnarBatch::from_data_points(&dps)
    }

    #[test]
    fn test_vectorized_aggregate() {
        let batch = make_batch();
        let result = VectorizedEngine::execute_aggregate(&batch, "usage", SimdAggFunc::Avg);
        assert!(result.is_some());
        if let FieldValue::Float(v) = result.unwrap() {
            assert!((v - 0.7).abs() < 0.01);
        }
    }

    #[test]
    fn test_vectorized_multi_aggregate() {
        let batch = make_batch();
        let results = VectorizedEngine::execute_multi_aggregate(
            &batch,
            &[
                ("usage", SimdAggFunc::Avg),
                ("usage", SimdAggFunc::Max),
                ("idle", SimdAggFunc::Min),
            ],
        );
        assert_eq!(results.len(), 3);
    }
}
