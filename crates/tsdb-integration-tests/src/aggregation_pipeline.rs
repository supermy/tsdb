//! 聚合管道集成测试
//!
//! 测试覆盖：
//! 1. LightAggregationPipeline → 数据摄入 → 聚合计算
//! 2. Aggregator → accumulate + finalize
//! 3. AggregationStore → 持久化存储 → 查询回读
//! 4. TimeseriesGenerator → SVG 图表生成

#![allow(dead_code, unused_imports)]

use std::collections::HashMap;
use std::sync::Arc;
use tsdb_aggregate::{
    aggregator::{AggregationResult, TimeDimension},
    AggregationStore, AggregationStoreManager, Aggregator, LightAggregationPipeline,
    PipelineConfig, TimeseriesGenerator,
};
use tsdb_chart::ChartType;
use tsdb_types::model::{DataPoint, FieldValue};

#[allow(dead_code)]
fn make_dp(measurement: &str, ts: i64, host: &str, usage: f64) -> DataPoint {
    let mut dp = DataPoint::new(measurement, ts);
    dp.tags.insert("host".to_string(), host.to_string());
    dp.fields
        .insert("usage".to_string(), FieldValue::Float(usage));
    dp
}

#[test]
fn test_pipeline_on_write_and_flush() {
    let dir = tempfile::TempDir::new().unwrap();
    let store_mgr = Arc::new(AggregationStoreManager::new(dir.path().to_path_buf()));
    let store = store_mgr.get_store("default").unwrap();

    let config = PipelineConfig {
        flush_interval_secs: 3600,
        buffer_size: 100,
        dimensions: vec![TimeDimension::Hour],
    };
    let pipeline = LightAggregationPipeline::new(config, store_mgr);

    let base_ts = 1_704_067_200_000_000;
    for i in 0..50i64 {
        let dp = make_dp(
            "cpu",
            base_ts + i * 60_000,
            "host_a",
            50.0 + (i as f64) * 0.5,
        );
        pipeline.on_write("default", &dp);
    }

    pipeline.flush_all().unwrap();

    assert_eq!(pipeline.buffered_count(), 0);

    let queried = store
        .query(TimeDimension::Hour, "cpu", 0, i64::MAX)
        .unwrap();
    assert!(!queried.is_empty());
}

#[test]
fn test_aggregator_accumulate_and_finalize() {
    let mut aggregator = Aggregator::new();

    let base_ts = 1_704_067_200_000_000;

    for dim in [TimeDimension::Hour, TimeDimension::Day, TimeDimension::Week] {
        for i in 0..20 {
            let dp = make_dp(
                "mem",
                base_ts
                    + i * (match dim {
                        TimeDimension::Hour => 3_600_000_000,
                        TimeDimension::Day => 86_400_000_000,
                        TimeDimension::Week => 604_800_000_000,
                        TimeDimension::Month => 2_592_000_000_000,
                    }),
                "server01",
                60.0 + (i as f64) * 2.0,
            );
            aggregator.accumulate(&dp);
        }

        let result = aggregator.finalize("mem", dim);
        assert!(!result.is_empty());

        aggregator.reset();
    }
}

#[test]
fn test_store_persistence_and_query() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = AggregationStore::open(dir.path(), "iot").unwrap();

    let base_ts = 1_700_000_000_000_000;
    let count_per_dim = 10;
    let dimensions = [TimeDimension::Hour, TimeDimension::Day];

    for &dim in &dimensions {
        for i in 0..count_per_dim {
            let result = AggregationResult {
                measurement: "sensor_temp".to_string(),
                dimension: dim,
                window_start: base_ts + i as i64 * 3_600_000_000,
                values: {
                    let mut m = HashMap::new();
                    m.insert("value".to_string(), 25.0 + (i as f64) * 0.3);
                    m
                },
            };
            store.write_result(&result).unwrap();
        }
    }

    for &dim in &dimensions {
        let results = store.query(dim, "sensor_temp", 0, i64::MAX).unwrap();
        assert!(results.len() >= count_per_dim / 2);
        for r in results {
            if let Some(v) = r.values.get("value") {
                assert!(*v >= 20.0 && *v <= 35.0);
            }
        }
    }
}

#[test]
fn test_multi_store_isolation() {
    let dir = tempfile::TempDir::new().unwrap();
    let s1 = AggregationStore::open(dir.path(), "stocks").unwrap();
    let s2 = AggregationStore::open(dir.path(), "weather").unwrap();

    let ts = 1_700_000_000_000_000;

    let result_s = AggregationResult {
        measurement: "price".to_string(),
        dimension: TimeDimension::Day,
        window_start: ts,
        values: {
            let mut m = HashMap::new();
            m.insert("close".to_string(), 150.0);
            m
        },
    };
    s1.write_result(&result_s).unwrap();

    let result_w = AggregationResult {
        measurement: "temp".to_string(),
        dimension: TimeDimension::Day,
        window_start: ts,
        values: {
            let mut m = HashMap::new();
            m.insert("celsius".to_string(), 22.5);
            m
        },
    };
    s2.write_result(&result_w).unwrap();

    let r1 = s1.query(TimeDimension::Day, "price", 0, i64::MAX).unwrap();
    let r2 = s2.query(TimeDimension::Day, "temp", 0, i64::MAX).unwrap();

    assert_eq!(r1.len(), 1);
    assert_eq!(r2.len(), 1);
    assert!(r1[0].values.contains_key("close"));
    assert!(r2[0].values.contains_key("celsius"));
}

#[test]
fn test_timeseries_generator_svg_output() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = AggregationStore::open(dir.path(), "chart_test").unwrap();

    let base_ts = 1_700_000_000_000_000;
    for i in 0..24 {
        let result = AggregationResult {
            measurement: "load".to_string(),
            dimension: TimeDimension::Hour,
            window_start: base_ts + i as i64 * 3_600_000_000,
            values: {
                let mut m = HashMap::new();
                m.insert("avg".to_string(), 30.0 + (i as f64 % 12.0));
                m
            },
        };
        store.write_result(&result).unwrap();
    }

    let svg = TimeseriesGenerator::generate_trend(
        &store,
        "chart_test",
        "load",
        TimeDimension::Hour,
        0,
        i64::MAX,
        ChartType::Area,
        "Load Trend",
    )
    .unwrap();

    assert!(svg.contains("<svg"));
    assert!(svg.contains("</svg>"));
    assert!(svg.contains("Load Trend") || svg.contains("load"));
}
