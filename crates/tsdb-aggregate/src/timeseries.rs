//! # 时序图生成器 — 按时间维度查看趋势
//!
//! 从聚合存储中读取指定业务、指标、维度的时间序列数据，
//! 生成 SVG 折线图/面积图，支持多指标叠加对比。
//!
//! ## 使用场景
//!
//! - 查看某指标的小时级/天级/周级/月级趋势
//! - 对比不同指标在同一时间范围内的走势
//! - 嵌入仪表盘或导出为独立图片

use crate::aggregator::{AggregationResult, TimeDimension};
use crate::store::AggregationStore;
use tsdb_chart::{ChartConfig, ChartType, SvgRenderer, TimeSeries, TimeSeriesChart};

pub struct TimeseriesGenerator;

impl TimeseriesGenerator {
    /// 从聚合存储生成时序趋势图
    ///
    /// # 参数
    ///
    /// - `store`: 聚合存储实例
    /// - `business`: 业务名称
    /// - `measurement`: 指标名称
    /// - `dimension`: 时间维度
    /// - `start_ts`: 起始时间戳（微秒）
    /// - `end_ts`: 结束时间戳（微秒）
    /// - `chart_type`: 图表类型
    /// - `title`: 图表标题
    #[allow(clippy::too_many_arguments)]
    pub fn generate_trend(
        store: &AggregationStore,
        _business: &str,
        measurement: &str,
        dimension: TimeDimension,
        start_ts: i64,
        end_ts: i64,
        chart_type: ChartType,
        title: &str,
    ) -> Result<String, String> {
        let results = store.query(dimension, measurement, start_ts, end_ts)?;

        if results.is_empty() {
            return Ok(Self::empty_chart(title, start_ts, end_ts));
        }

        let field_names = Self::extract_field_names(&results);
        let config = ChartConfig {
            title: title.to_string(),
            chart_type,
            x_label: dimension.name().to_string(),
            y_label: measurement.to_string(),
            show_points: results.len() < 50,
            ..Default::default()
        };

        let mut chart = TimeSeriesChart::new(config);

        for field_name in &field_names {
            let mut series = TimeSeries::new(format!("{}_{}", measurement, field_name));
            for result in &results {
                if let Some(val) = result.values.get(field_name) {
                    series.add_point(result.window_start, *val);
                }
            }
            if !series.is_empty() {
                chart.add_series(series);
            }
        }

        Ok(SvgRenderer::render(&chart))
    }

    /// 生成多指标对比趋势图
    ///
    /// 将多个 measurement 的数据叠加在同一张图中
    pub fn generate_comparison(
        store: &AggregationStore,
        _business: &str,
        measurements: &[&str],
        dimension: TimeDimension,
        start_ts: i64,
        end_ts: i64,
        title: &str,
    ) -> Result<String, String> {
        let config = ChartConfig {
            title: title.to_string(),
            chart_type: ChartType::Line,
            x_label: dimension.name().to_string(),
            y_label: "Value".to_string(),
            show_points: false,
            ..Default::default()
        };

        let mut chart = TimeSeriesChart::new(config);
        let mut total_points = 0;

        for measurement in measurements {
            let results = store.query(dimension, measurement, start_ts, end_ts)?;
            for result in &results {
                for (field_name, val) in &result.values {
                    let mut series = TimeSeries::new(format!("{}.{}", measurement, field_name));
                    series.add_point(result.window_start, *val);
                    total_points += 1;

                    let downsampled = series.downsample(1000);
                    chart.add_series(downsampled);
                }
            }
        }

        if total_points == 0 {
            return Ok(Self::empty_chart(title, start_ts, end_ts));
        }

        Ok(SvgRenderer::render(&chart))
    }

    /// 生成多维度叠加趋势图
    ///
    /// 同一指标在不同时间维度（hour/day/week/month）下的趋势对比
    pub fn generate_multi_dimension(
        store: &AggregationStore,
        _business: &str,
        measurement: &str,
        dimensions: &[TimeDimension],
        start_ts: i64,
        end_ts: i64,
        title: &str,
    ) -> Result<String, String> {
        let config = ChartConfig {
            title: title.to_string(),
            chart_type: ChartType::Line,
            x_label: "Time".to_string(),
            y_label: measurement.to_string(),
            show_points: false,
            ..Default::default()
        };

        let mut chart = TimeSeriesChart::new(config);
        let mut has_data = false;

        for &dim in dimensions {
            let results = store.query(dim, measurement, start_ts, end_ts)?;
            for result in &results {
                for (field_name, val) in &result.values {
                    let mut series = TimeSeries::new(format!("{}_{}", dim.name(), field_name));
                    series.add_point(result.window_start, *val);
                    has_data = true;
                    chart.add_series(series);
                }
            }
        }

        if !has_data {
            return Ok(Self::empty_chart(title, start_ts, end_ts));
        }

        Ok(SvgRenderer::render(&chart))
    }

    /// 从聚合结果中提取所有字段名
    fn extract_field_names(results: &[AggregationResult]) -> Vec<String> {
        let mut names = std::collections::BTreeSet::new();
        for result in results {
            for key in result.values.keys() {
                names.insert(key.clone());
            }
        }
        names.into_iter().collect()
    }

    /// 生成空数据时的占位图
    fn empty_chart(title: &str, _start_ts: i64, _end_ts: i64) -> String {
        let config = ChartConfig {
            title: format!("{} (no data)", title),
            chart_type: ChartType::Line,
            ..Default::default()
        };
        let chart = TimeSeriesChart::new(config);
        SvgRenderer::render(&chart)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregator::AggregationResult;
    use std::collections::HashMap;

    #[test]
    fn test_generate_trend_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = AggregationStore::open(dir.path(), "test_trend").unwrap();

        let svg = TimeseriesGenerator::generate_trend(
            &store,
            "test",
            "cpu",
            TimeDimension::Hour,
            0,
            i64::MAX,
            ChartType::Line,
            "CPU Trend",
        )
        .unwrap();

        assert!(svg.contains("<svg"));
        assert!(svg.contains("no data"));
    }

    #[test]
    fn test_generate_trend_with_data() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = AggregationStore::open(dir.path(), "test_trend_data").unwrap();

        for i in 0..5 {
            let result = AggregationResult {
                measurement: "cpu".to_string(),
                dimension: TimeDimension::Hour,
                window_start: (1713158400 + i * 3600) * 1_000_000,
                values: {
                    let mut m = HashMap::new();
                    m.insert("usage".to_string(), 50.0 + i as f64 * 5.0);
                    m
                },
            };
            store.write_result(&result).unwrap();
        }

        let svg = TimeseriesGenerator::generate_trend(
            &store,
            "test",
            "cpu",
            TimeDimension::Hour,
            0,
            i64::MAX,
            ChartType::Line,
            "CPU Hourly Trend",
        )
        .unwrap();

        assert!(svg.contains("<svg"));
        assert!(!svg.contains("no data"));
    }

    #[test]
    fn test_generate_trend_area_chart() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = AggregationStore::open(dir.path(), "test_area").unwrap();

        for i in 0..3 {
            let result = AggregationResult {
                measurement: "mem".to_string(),
                dimension: TimeDimension::Day,
                window_start: (1713158400 + i * 86400) * 1_000_000,
                values: {
                    let mut m = HashMap::new();
                    m.insert("used".to_string(), 60.0 + i as f64 * 2.0);
                    m
                },
            };
            store.write_result(&result).unwrap();
        }

        let svg = TimeseriesGenerator::generate_trend(
            &store,
            "test",
            "mem",
            TimeDimension::Day,
            0,
            i64::MAX,
            ChartType::Area,
            "Memory Daily Trend",
        )
        .unwrap();

        assert!(svg.contains("<svg"));
    }
}
