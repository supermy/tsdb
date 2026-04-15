use crate::parser::SqlParser;
use crate::plan::{PlanNode, QueryPlanner, AggFunc, AggExpr};
use tsdb_types::model::{DataPoint, FieldValue};
use tsdb_core::storage::StorageEngine;
use std::collections::HashMap;

pub struct QueryEngine {
    parser: SqlParser,
}

#[derive(Debug)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<FieldValue>>,
}

impl QueryEngine {
    pub fn new() -> Self {
        Self {
            parser: SqlParser::new(),
        }
    }

    pub fn execute(&self, sql: &str, storage: &StorageEngine) -> Result<QueryResult, QueryError> {
        let parsed = self.parser.parse(sql)
            .map_err(|e| QueryError::Parse(e.to_string()))?;

        let plan = QueryPlanner::plan(&parsed);
        self.execute_plan(&plan, storage)
    }

    fn execute_plan(&self, plan: &PlanNode, storage: &StorageEngine) -> Result<QueryResult, QueryError> {
        match plan {
            PlanNode::Scan { measurement, time_range, tag_filters } => {
                let (start, end) = time_range.unwrap_or((
                    0,
                    chrono::Utc::now().timestamp_micros(),
                ));

                let tags: std::collections::BTreeMap<String, String> = tag_filters.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();

                let data_points = storage.read_range(measurement, &tags, start, end)
                    .map_err(|e| QueryError::Execution(e.to_string()))?;

                if data_points.is_empty() {
                    return Ok(QueryResult {
                        columns: vec!["time".to_string()],
                        rows: vec![],
                    });
                }

                let mut field_names: Vec<String> = data_points.first()
                    .map(|dp| dp.fields.keys().cloned().collect())
                    .unwrap_or_default();
                field_names.sort();
                let mut columns = vec!["time".to_string(), "measurement".to_string()];
                columns.extend(field_names.clone());

                let rows: Vec<Vec<FieldValue>> = data_points.into_iter().map(|dp| {
                    let mut row = vec![
                        FieldValue::Integer(dp.timestamp),
                        FieldValue::String(dp.measurement),
                    ];
                    for name in &field_names {
                        row.push(dp.fields.get(name).cloned().unwrap_or(FieldValue::String(String::new())));
                    }
                    row
                }).collect();

                Ok(QueryResult { columns, rows })
            }

            PlanNode::Aggregate { input, aggs, group_by } => {
                let input_result = self.execute_plan(input, storage)?;

                let mut result_columns = group_by.clone();
                for agg in aggs {
                    let name = agg.alias.clone().unwrap_or_else(|| {
                        format!("{:?}({})", agg.func, agg.field)
                    });
                    result_columns.push(name);
                }

                let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
                for (row_idx, row) in input_result.rows.iter().enumerate() {
                    let key: String = group_by.iter()
                        .filter_map(|name| {
                            input_result.columns.iter().position(|c| c == name)
                                .map(|idx| format!("{:?}", row[idx]))
                        })
                        .collect::<Vec<_>>()
                        .join("|");
                    groups.entry(key).or_default().push(row_idx);
                }

                let mut result_rows = Vec::new();
                for (key_str, row_indices) in groups {
                    let mut result_row = Vec::new();
                    if let Some(first_idx) = row_indices.first() {
                        for name in group_by {
                            if let Some(col_idx) = input_result.columns.iter().position(|c| c == name) {
                                result_row.push(input_result.rows[*first_idx][col_idx].clone());
                            }
                        }
                    }

                    for agg in aggs {
                        let col_idx = input_result.columns.iter().position(|c| c == &agg.field);
                        let values: Vec<&FieldValue> = row_indices.iter()
                            .filter_map(|&idx| col_idx.map(|ci| &input_result.rows[idx][ci]))
                            .collect();

                        let agg_value = compute_aggregate(agg.func, &values);
                        result_row.push(agg_value);
                    }
                    result_rows.push(result_row);
                }

                Ok(QueryResult {
                    columns: result_columns,
                    rows: result_rows,
                })
            }

            PlanNode::Limit { input, count } => {
                let mut result = self.execute_plan(input, storage)?;
                result.rows.truncate(*count);
                Ok(result)
            }

            PlanNode::Sort { input, field, desc } => {
                let mut result = self.execute_plan(input, storage)?;
                let col_idx = result.columns.iter().position(|c| c == field);
                if let Some(idx) = col_idx {
                    result.rows.sort_by(|a, b| {
                        let cmp = compare_field_values(&a[idx], &b[idx]);
                        if *desc { cmp.reverse() } else { cmp }
                    });
                }
                Ok(result)
            }

            PlanNode::Project { input, fields } => {
                let input_result = self.execute_plan(input, storage)?;
                let indices: Vec<usize> = fields.iter()
                    .filter_map(|f| input_result.columns.iter().position(|c| c == f))
                    .collect();

                let columns: Vec<String> = indices.iter()
                    .map(|&i| input_result.columns[i].clone())
                    .collect();

                let rows: Vec<Vec<FieldValue>> = input_result.rows.into_iter()
                    .map(|row| indices.iter().map(|&i| row[i].clone()).collect())
                    .collect();

                Ok(QueryResult { columns, rows })
            }

            PlanNode::Filter { input, predicate } => {
                let input_result = self.execute_plan(input, storage)?;
                let col_idx = input_result.columns.iter().position(|c| c == &predicate.field);
                let rows: Vec<Vec<FieldValue>> = if let Some(idx) = col_idx {
                    input_result.rows.into_iter()
                        .filter(|row| apply_filter(&row[idx], predicate))
                        .collect()
                } else {
                    input_result.rows
                };
                Ok(QueryResult {
                    columns: input_result.columns,
                    rows,
                })
            }
        }
    }
}

fn compute_aggregate(func: AggFunc, values: &[&FieldValue]) -> FieldValue {
    match func {
        AggFunc::Count => FieldValue::Integer(values.len() as i64),
        AggFunc::Sum => {
            let sum: f64 = values.iter().filter_map(|v| v.as_f64()).sum();
            FieldValue::Float(sum)
        }
        AggFunc::Avg => {
            let sum: f64 = values.iter().filter_map(|v| v.as_f64()).sum();
            let count = values.iter().filter(|v| v.as_f64().is_some()).count();
            if count > 0 {
                FieldValue::Float(sum / count as f64)
            } else {
                FieldValue::Float(0.0)
            }
        }
        AggFunc::Min => {
            let min = values.iter()
                .filter_map(|v| v.as_f64())
                .fold(f64::INFINITY, f64::min);
            FieldValue::Float(min)
        }
        AggFunc::Max => {
            let max = values.iter()
                .filter_map(|v| v.as_f64())
                .fold(f64::NEG_INFINITY, f64::max);
            FieldValue::Float(max)
        }
        AggFunc::First => values.first().cloned().cloned().unwrap_or(FieldValue::Integer(0)),
        AggFunc::Last => values.last().cloned().cloned().unwrap_or(FieldValue::Integer(0)),
    }
}

fn compare_field_values(a: &FieldValue, b: &FieldValue) -> std::cmp::Ordering {
    match (a.as_f64(), b.as_f64()) {
        (Some(va), Some(vb)) => va.partial_cmp(&vb).unwrap_or(std::cmp::Ordering::Equal),
        _ => std::cmp::Ordering::Equal,
    }
}

fn apply_filter(value: &FieldValue, predicate: &crate::plan::FilterPredicate) -> bool {
    match predicate.op {
        crate::plan::FilterOp::Eq => {
            match (&value, &predicate.value) {
                (FieldValue::Float(a), FieldValue::Float(b)) => (a - b).abs() < f64::EPSILON,
                (FieldValue::Integer(a), FieldValue::Integer(b)) => a == b,
                (FieldValue::String(a), FieldValue::String(b)) => a == b,
                (FieldValue::Boolean(a), FieldValue::Boolean(b)) => a == b,
                _ => true,
            }
        }
        _ => true,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("parse error: {0}")]
    Parse(String),
    #[error("execution error: {0}")]
    Execution(String),
}
