use tsdb_core::storage::{StorageEngine, CfConfig};
use tsdb_types::model::{DataPoint, FieldValue};
use std::collections::BTreeMap;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let dir = tempfile::TempDir::new()?;
    let config = CfConfig::default();
    let engine = StorageEngine::open(dir.path(), config)?;

    println!("=== TSDB Performance Benchmark ===\n");

    let device_count = 100;
    let metrics_per_device = 10;
    let interval_secs: i64 = 10;
    let duration_hours: i64 = 1;
    let points_per_device = (duration_hours * 3600) / interval_secs;
    let total_points = device_count * points_per_device;

    println!("Configuration:");
    println!("  Devices: {}", device_count);
    println!("  Metrics per device: {}", metrics_per_device);
    println!("  Interval: {}s", interval_secs);
    println!("  Duration: {}h", duration_hours);
    println!("  Total data points: {}\n", total_points);

    let base_ts: i64 = 1704067200_000_000;
    let mut batch = Vec::with_capacity(1000);
    let mut written = 0usize;

    let start = Instant::now();

    for device_id in 0..device_count {
        for point_idx in 0..points_per_device {
            let ts = base_ts + point_idx * interval_secs * 1_000_000;

            let mut tags = BTreeMap::new();
            tags.insert("hostname".to_string(), format!("host_{}", device_id));
            tags.insert("region".to_string(), format!("region_{}", device_id % 5));
            tags.insert("datacenter".to_string(), format!("dc_{}", device_id % 3));

            let mut dp = DataPoint::new("cpu", ts);
            dp.tags = tags;

            for metric in 0..metrics_per_device {
                let value = (device_id as f64 * 0.1 + metric as f64 * 0.01 + point_idx as f64 * 0.001) % 100.0;
                dp.fields.insert(format!("metric_{}", metric), FieldValue::Float(value));
            }

            batch.push(dp);
            written += 1;

            if batch.len() >= 1000 {
                engine.write_batch(&batch)?;
                batch.clear();

                if written % 100_000 == 0 {
                    let elapsed = start.elapsed().as_secs_f64();
                    let rate = written as f64 / elapsed;
                    println!("  Written {} points ({:.0} points/sec)", written, rate);
                }
            }
        }
    }

    if !batch.is_empty() {
        engine.write_batch(&batch)?;
    }

    let write_elapsed = start.elapsed();
    let write_rate = written as f64 / write_elapsed.as_secs_f64();

    println!("\n--- Write Results ---");
    println!("  Total points: {}", written);
    println!("  Write time: {:.2}s", write_elapsed.as_secs_f64());
    println!("  Write rate: {:.0} points/sec", write_rate);

    let mut tags = BTreeMap::new();
    tags.insert("hostname".to_string(), "host_0".to_string());

    let query_start = Instant::now();
    let results = engine.read_range("cpu", &tags, base_ts, base_ts + 60_000_000)?;
    let query_elapsed = query_start.elapsed();

    println!("\n--- Query Results ---");
    println!("  Query time: {:.2}ms", query_elapsed.as_secs_f64() * 1000.0);
    println!("  Results: {} points", results.len());

    println!("\n=== Benchmark Complete ===");

    Ok(())
}
