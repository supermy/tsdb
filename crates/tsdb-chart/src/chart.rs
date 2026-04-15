use crate::series::TimeSeries;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ChartType {
    Line,
    Area,
    Bar,
    Scatter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub chart_type: ChartType,
    pub x_label: String,
    pub y_label: String,
    pub show_legend: bool,
    pub show_grid: bool,
    pub show_points: bool,
    pub margin: Margin,
    pub colors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Margin {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

impl Default for ChartConfig {
    fn default() -> Self {
        Self {
            title: String::new(),
            width: 800,
            height: 400,
            chart_type: ChartType::Line,
            x_label: "Time".to_string(),
            y_label: "Value".to_string(),
            show_legend: true,
            show_grid: true,
            show_points: false,
            margin: Margin {
                top: 30,
                right: 30,
                bottom: 50,
                left: 60,
            },
            colors: vec![
                "#4e79a7".to_string(),
                "#f28e2b".to_string(),
                "#e15759".to_string(),
                "#76b7b2".to_string(),
                "#59a14f".to_string(),
                "#edc948".to_string(),
                "#b07aa1".to_string(),
                "#ff9da7".to_string(),
            ],
        }
    }
}

pub struct TimeSeriesChart {
    pub config: ChartConfig,
    pub series: Vec<TimeSeries>,
}

impl TimeSeriesChart {
    pub fn new(config: ChartConfig) -> Self {
        Self {
            config,
            series: Vec::new(),
        }
    }

    pub fn add_series(&mut self, series: TimeSeries) {
        self.series.push(series);
    }

    pub fn data_bounds(&self) -> (f64, f64, f64, f64) {
        let mut min_ts = f64::INFINITY;
        let mut max_ts = f64::NEG_INFINITY;
        let mut min_val = f64::INFINITY;
        let mut max_val = f64::NEG_INFINITY;

        for s in &self.series {
            if s.is_empty() {
                continue;
            }
            let ts_min = *s.timestamps.first().unwrap_or(&0) as f64;
            let ts_max = *s.timestamps.last().unwrap_or(&0) as f64;
            min_ts = min_ts.min(ts_min);
            max_ts = max_ts.max(ts_max);
            min_val = min_val.min(s.min_value());
            max_val = max_val.max(s.max_value());
        }

        if min_ts == f64::INFINITY {
            min_ts = 0.0;
        }
        if max_ts == f64::NEG_INFINITY {
            max_ts = 1.0;
        }
        if min_val == f64::INFINITY {
            min_val = 0.0;
        }
        if max_val == f64::NEG_INFINITY {
            max_val = 1.0;
        }

        let val_range = max_val - min_val;
        let val_padding = val_range * 0.05;
        min_val -= val_padding;
        max_val += val_padding;

        (min_ts, max_ts, min_val, max_val)
    }

    pub fn to_json(&self) -> String {
        let data: Vec<serde_json::Value> = self.series.iter().map(|s| {
            let points: Vec<Vec<f64>> = s.timestamps.iter().zip(s.values.iter())
                .map(|(&ts, &v)| vec![ts as f64, v])
                .collect();
            serde_json::json!({
                "name": s.name,
                "points": points,
            })
        }).collect();

        serde_json::json!({
            "config": self.config,
            "series": data,
        }).to_string()
    }
}
