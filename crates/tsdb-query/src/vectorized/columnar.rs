use tsdb_types::model::FieldValue;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Column {
    Float(Vec<f64>),
    Integer(Vec<i64>),
    String(Vec<String>),
    Boolean(Vec<bool>),
    Timestamp(Vec<i64>),
}

impl Column {
    pub fn len(&self) -> usize {
        match self {
            Column::Float(v) => v.len(),
            Column::Integer(v) => v.len(),
            Column::String(v) => v.len(),
            Column::Boolean(v) => v.len(),
            Column::Timestamp(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_float_slice(&self) -> Option<&[f64]> {
        match self {
            Column::Float(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_int_slice(&self) -> Option<&[i64]> {
        match self {
            Column::Integer(v) => Some(v),
            _ => None,
        }
    }

    pub fn push_field_value(&mut self, fv: &FieldValue) {
        match (self, fv) {
            (Column::Float(v), FieldValue::Float(f)) => v.push(*f),
            (Column::Integer(v), FieldValue::Integer(i)) => v.push(*i),
            (Column::String(v), FieldValue::String(s)) => v.push(s.clone()),
            (Column::Boolean(v), FieldValue::Boolean(b)) => v.push(*b),
            _ => {}
        }
    }
}

#[derive(Debug, Clone)]
pub struct ColumnarBatch {
    pub columns: HashMap<String, Column>,
    pub row_count: usize,
}

impl ColumnarBatch {
    pub fn new() -> Self {
        Self {
            columns: HashMap::new(),
            row_count: 0,
        }
    }

    pub fn from_data_points(data_points: &[tsdb_types::model::DataPoint]) -> Self {
        if data_points.is_empty() {
            return Self::new();
        }

        let mut columns: HashMap<String, Column> = HashMap::new();
        columns.insert("time".to_string(), Column::Timestamp(Vec::with_capacity(data_points.len())));
        columns.insert("measurement".to_string(), Column::String(Vec::with_capacity(data_points.len())));

        let first = &data_points[0];
        for (name, fv) in &first.fields {
            let col = match fv {
                FieldValue::Float(_) => Column::Float(Vec::with_capacity(data_points.len())),
                FieldValue::Integer(_) => Column::Integer(Vec::with_capacity(data_points.len())),
                FieldValue::String(_) => Column::String(Vec::with_capacity(data_points.len())),
                FieldValue::Boolean(_) => Column::Boolean(Vec::with_capacity(data_points.len())),
            };
            columns.insert(name.clone(), col);
        }

        for dp in data_points {
            if let Some(Column::Timestamp(v)) = columns.get_mut("time") {
                v.push(dp.timestamp);
            }
            if let Some(Column::String(v)) = columns.get_mut("measurement") {
                v.push(dp.measurement.clone());
            }
            for (name, fv) in &dp.fields {
                if let Some(col) = columns.get_mut(name) {
                    col.push_field_value(fv);
                }
            }
        }

        Self {
            row_count: data_points.len(),
            columns,
        }
    }

    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.get(name)
    }

    pub fn column_names(&self) -> Vec<&str> {
        self.columns.keys().map(|s| s.as_str()).collect()
    }

    pub fn filter(&self, predicate: &dyn Fn(&ColumnarBatch, usize) -> bool) -> ColumnarBatch {
        let mut indices = Vec::new();
        for i in 0..self.row_count {
            if predicate(self, i) {
                indices.push(i);
            }
        }

        let mut new_columns = HashMap::new();
        for (name, col) in &self.columns {
            let new_col = col.select(&indices);
            new_columns.insert(name.clone(), new_col);
        }

        ColumnarBatch {
            columns: new_columns,
            row_count: indices.len(),
        }
    }

    pub fn project(&self, columns: &[String]) -> ColumnarBatch {
        let mut new_columns = HashMap::new();
        for name in columns {
            if let Some(col) = self.columns.get(name) {
                new_columns.insert(name.clone(), col.clone());
            }
        }
        ColumnarBatch {
            columns: new_columns,
            row_count: self.row_count,
        }
    }
}

impl Column {
    fn select(&self, indices: &[usize]) -> Column {
        match self {
            Column::Float(v) => Column::Float(indices.iter().map(|&i| v[i]).collect()),
            Column::Integer(v) => Column::Integer(indices.iter().map(|&i| v[i]).collect()),
            Column::String(v) => Column::String(indices.iter().map(|&i| v[i].clone()).collect()),
            Column::Boolean(v) => Column::Boolean(indices.iter().map(|&i| v[i]).collect()),
            Column::Timestamp(v) => Column::Timestamp(indices.iter().map(|&i| v[i]).collect()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsdb_types::model::DataPoint;

    #[test]
    fn test_columnar_batch_from_data_points() {
        let dps = vec![
            {
                let mut dp = DataPoint::new("cpu", 1000);
                dp.fields.insert("usage".to_string(), FieldValue::Float(0.5));
                dp
            },
            {
                let mut dp = DataPoint::new("cpu", 2000);
                dp.fields.insert("usage".to_string(), FieldValue::Float(0.7));
                dp
            },
        ];

        let batch = ColumnarBatch::from_data_points(&dps);
        assert_eq!(batch.row_count, 2);
        assert!(batch.column("usage").is_some());
        assert!(batch.column("time").is_some());
    }

    #[test]
    fn test_columnar_filter() {
        let dps = vec![
            {
                let mut dp = DataPoint::new("cpu", 1000);
                dp.fields.insert("usage".to_string(), FieldValue::Float(0.5));
                dp
            },
            {
                let mut dp = DataPoint::new("cpu", 2000);
                dp.fields.insert("usage".to_string(), FieldValue::Float(0.9));
                dp
            },
        ];

        let batch = ColumnarBatch::from_data_points(&dps);
        let filtered = batch.filter(&|b, i| {
            if let Some(Column::Float(v)) = b.column("usage") {
                v[i] > 0.6
            } else {
                false
            }
        });
        assert_eq!(filtered.row_count, 1);
    }
}
