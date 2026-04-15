use serde::{Deserialize, Serialize};
use tsdb_types::model::FieldValue;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusinessMetric {
    pub name: String,
    pub current: f64,
    pub previous: f64,
    pub unit: String,
    pub trend: Trend,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Trend {
    Up,
    Down,
    Stable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusinessDashboard {
    pub metrics: Vec<BusinessMetric>,
    pub total_points: usize,
    pub measurements: Vec<String>,
}

impl BusinessDashboard {
    pub fn new() -> Self {
        Self { metrics: Vec::new(), total_points: 0, measurements: Vec::new() }
    }

    pub fn from_data_points(data_points: &[tsdb_types::model::DataPoint]) -> BusinessDashboard {
        let mut metrics = Vec::new();
        let mut field_values: std::collections::HashMap<String, Vec<f64>> = std::collections::HashMap::new();

        for dp in data_points {
            for (field_name, field_value) in &dp.fields {
                if let Some(v) = field_value.as_f64() {
                    field_values.entry(field_name.clone()).or_default().push(v);
                }
            }
        }

        for (field_name, values) in &field_values {
            if values.len() >= 2 {
                let current = *values.last().unwrap_or(&0.0);
                let prev = *values.get(values.len().saturating_sub(2)).unwrap_or(&0.0);
                let change = if prev > 0.0 { (current - prev) / prev } else { 0.0 };
                let trend = if change > 0.05 { Trend::Up } else if change < -0.05 { Trend::Down } else { Trend::Stable };

                metrics.push(BusinessMetric {
                    name: field_name.clone(),
                    current,
                    previous: prev,
                    unit: "value".to_string(),
                    trend,
                });
            }
        }

        BusinessDashboard { metrics, total_points: data_points.len(), measurements: Self::extract_measurements(data_points) }
    }

    pub fn from_query_result(columns: &[String], rows: &[Vec<FieldValue>]) -> BusinessDashboard {
        let mut metrics = Vec::new();
        let mut col_values: std::collections::HashMap<String, Vec<f64>> = std::collections::HashMap::new();

        for row in rows {
            for (i, val) in row.iter().enumerate() {
                if let Some(col_name) = columns.get(i) {
                    if let Some(v) = val.as_f64() {
                        col_values.entry(col_name.clone()).or_default().push(v);
                    }
                }
            }
        }

        for (col_name, values) in &col_values {
            if values.len() >= 2 {
                let current = *values.last().unwrap_or(&0.0);
                let prev = *values.get(values.len().saturating_sub(2)).unwrap_or(&0.0);
                let change = if prev > 0.0 { (current - prev) / prev } else { 0.0 };
                let trend = if change > 0.05 { Trend::Up } else if change < -0.05 { Trend::Down } else { Trend::Stable };

                metrics.push(BusinessMetric {
                    name: col_name.clone(),
                    current,
                    previous: prev,
                    unit: "value".to_string(),
                    trend,
                });
            }
        }

        BusinessDashboard {
            metrics,
            total_points: rows.len(),
            measurements: columns.to_vec(),
        }
    }

    fn extract_measurements(dps: &[tsdb_types::model::DataPoint]) -> Vec<String> {
        dps.iter().map(|dp| dp.measurement.clone()).collect::<std::collections::HashSet<_>>().into_iter().collect()
    }

    pub fn summary_json(&self) -> serde_json::Value {
        let mut cards = Vec::new();
        for m in &self.metrics {
            let change_pct = if m.previous > 0.0 {
                ((m.current - m.previous) / m.previous * 100.0)
            } else {
                0.0
            };
            cards.push(serde_json::json!({
                "name": m.name,
                "value": m.current,
                "previous": m.previous,
                "change_pct": format!("{:.1}%", change_pct),
                "trend": match m.trend {
                    Trend::Up => "up",
                    Trend::Down => "down",
                    Trend::Stable => "stable",
                },
            }));
        }

        serde_json::json!({
            "type": "business_dashboard",
            "total_data_points": self.total_points,
            "measurements_count": self.measurements.len(),
            "metrics": cards,
        })
    }

    pub fn to_chart_config(&self) -> tsdb_chart::ChartConfig {
        tsdb_chart::ChartConfig {
            title: "Business Metrics".to_string(),
            chart_type: tsdb_chart::ChartType::Bar,
            show_legend: true,
            show_grid: true,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsdb_types::model::{DataPoint, FieldValue};

    #[test]
    fn test_business_dashboard_from_data() {
        let dps = vec![
            DataPoint::new("cpu", 1000).with_field("usage", FieldValue::Float(0.5)),
            DataPoint::new("cpu", 2000).with_field("usage", FieldValue::Float(0.7)),
            DataPoint::new("cpu", 3000).with_field("usage", FieldValue::Float(0.9)),
            DataPoint::new("mem", 1000).with_field("used", FieldValue::Float(40.0)),
            DataPoint::new("mem", 2000).with_field("used", FieldValue::Float(60.0)),
        ];

        let dash = BusinessDashboard::from_data_points(&dps);
        assert_eq!(dash.total_points, 5);
        assert!(dash.summary_json()["metrics"].as_array().unwrap().len() > 0);

        let json = dash.summary_json();
        assert_eq!(json["type"], "business_dashboard");
    }
}
