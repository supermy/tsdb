use crate::vectorized::columnar::{Column, ColumnarBatch};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SimdAggFunc {
    Sum,
    Avg,
    Min,
    Max,
    Count,
}

pub struct SimdAggregator;

impl SimdAggregator {
    pub fn aggregate(batch: &ColumnarBatch, column_name: &str, func: SimdAggFunc) -> Option<f64> {
        let col = batch.column(column_name)?;
        match col {
            Column::Float(v) => Self::aggregate_f64(v, func),
            Column::Integer(v) => Self::aggregate_i64(v, func),
            Column::Timestamp(v) => Self::aggregate_i64(v, func),
            _ => None,
        }
    }

    pub fn aggregate_f64(values: &[f64], func: SimdAggFunc) -> Option<f64> {
        if values.is_empty() {
            return None;
        }

        let chunk_size = 4;
        let chunks = values.chunks(chunk_size);

        match func {
            SimdAggFunc::Sum => {
                let mut sum = [0.0f64; 4];
                for chunk in chunks {
                    let len = chunk.len();
                    for i in 0..len {
                        sum[i % 4] += chunk[i];
                    }
                }
                Some(sum.iter().sum())
            }
            SimdAggFunc::Avg => {
                let sum = Self::aggregate_f64(values, SimdAggFunc::Sum)?;
                Some(sum / values.len() as f64)
            }
            SimdAggFunc::Min => {
                let mut min = values[0];
                for chunk in chunks {
                    for &v in chunk {
                        if v < min {
                            min = v;
                        }
                    }
                }
                Some(min)
            }
            SimdAggFunc::Max => {
                let mut max = values[0];
                for chunk in chunks {
                    for &v in chunk {
                        if v > max {
                            max = v;
                        }
                    }
                }
                Some(max)
            }
            SimdAggFunc::Count => Some(values.len() as f64),
        }
    }

    pub fn aggregate_i64(values: &[i64], func: SimdAggFunc) -> Option<f64> {
        if values.is_empty() {
            return None;
        }

        match func {
            SimdAggFunc::Sum => Some(values.iter().sum::<i64>() as f64),
            SimdAggFunc::Avg => Some(values.iter().sum::<i64>() as f64 / values.len() as f64),
            SimdAggFunc::Min => Some(*values.iter().min()? as f64),
            SimdAggFunc::Max => Some(*values.iter().max()? as f64),
            SimdAggFunc::Count => Some(values.len() as f64),
        }
    }

    pub fn group_aggregate(
        batch: &ColumnarBatch,
        group_column: &str,
        agg_column: &str,
        func: SimdAggFunc,
    ) -> Vec<(String, f64)> {
        let group_col = match batch.column(group_column) {
            Some(Column::String(v)) => v,
            _ => return Vec::new(),
        };

        let agg_col = match batch.column(agg_column) {
            Some(Column::Float(v)) => v,
            _ => return Vec::new(),
        };

        let mut groups: HashMap<String, Vec<f64>> = HashMap::new();
        for (i, key) in group_col.iter().enumerate() {
            if i < agg_col.len() {
                groups.entry(key.clone()).or_default().push(agg_col[i]);
            }
        }

        let mut results: Vec<(String, f64)> = groups
            .into_iter()
            .filter_map(|(key, values)| Self::aggregate_f64(&values, func).map(|v| (key, v)))
            .collect();

        results.sort_by(|a, b| a.0.cmp(&b.0));
        results
    }
}

use std::collections::HashMap;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simd_sum() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let result = SimdAggregator::aggregate_f64(&values, SimdAggFunc::Sum);
        assert_eq!(result, Some(15.0));
    }

    #[test]
    fn test_simd_avg() {
        let values = vec![2.0, 4.0, 6.0];
        let result = SimdAggregator::aggregate_f64(&values, SimdAggFunc::Avg);
        assert_eq!(result, Some(4.0));
    }

    #[test]
    fn test_simd_min_max() {
        let values = vec![3.0, 1.0, 4.0, 1.5, 9.0];
        assert_eq!(
            SimdAggregator::aggregate_f64(&values, SimdAggFunc::Min),
            Some(1.0)
        );
        assert_eq!(
            SimdAggregator::aggregate_f64(&values, SimdAggFunc::Max),
            Some(9.0)
        );
    }

    #[test]
    fn test_simd_count() {
        let values = vec![1.0, 2.0, 3.0];
        assert_eq!(
            SimdAggregator::aggregate_f64(&values, SimdAggFunc::Count),
            Some(3.0)
        );
    }
}
