//! # TSDB 真实数据性能基准测试
//!
//! 模拟 TSBS DevOps 场景，全面测试 TSDB 各子系统性能：
//! - 写入吞吐量（单点/批量/MergedBlock）
//! - 查询延迟（范围查询/精确查询/聚合查询）
//! - 压缩效率（Delta/Gorilla/Dictionary）
//! - 向量化 vs 标量聚合对比
//! - 轻度汇总管道吞吐量
//! - 多业务DB隔离开销

use tsdb_core::storage::{StorageEngine, cf_manager::CfConfig, MultiDbManager};
use tsdb_compress::codec::{BlockCodec, Codec, DataBlock};
use tsdb_compress::gorilla::GorillaEncoder;
use tsdb_compress::delta::DeltaEncoder;
use tsdb_compress::dictionary::DictionaryEncoder;
use tsdb_index::{IndexManager, inverted::InvertedIndex};
use tsdb_aggregate::{pipeline::LightAggregationPipeline, pipeline::PipelineConfig, store::AggregationStoreManager, aggregator::TimeDimension};
use tsdb_query::QueryEngine;
use tsdb_types::model::{DataPoint, FieldValue, Tags};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║          TSDB 真实数据性能基准测试 v0.2.0                    ║");
    println!("║          TSBS DevOps Scenario Simulation                    ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    let dir = tempfile::TempDir::new()?;
    let config = CfConfig::default();
    let engine = StorageEngine::open(dir.path(), config)?;

    // ==================== 1. 写入性能测试 ====================
    bench_write_throughput(&engine)?;

    // ==================== 2. 查询性能测试 ====================
    bench_query_latency(&engine)?;

    // ==================== 3. 压缩效率测试 ====================
    bench_compression()?;

    // ==================== 4. 向量化 vs 标量聚合对比 ====================
    bench_vectorized_vs_scalar(&engine)?;

    // ==================== 5. 轻度汇总管道性能 ====================
    bench_aggregation_pipeline()?;

    // ==================== 6. 多业务DB隔离开销 ====================
    bench_multi_db_isolation()?;

    // ==================== 7. 索引性能 ====================
    bench_index_performance()?;

    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║                    全部测试完成                              ║");
    println!("╚══════════════════════════════════════════════════════════════╝");

    Ok(())
}

/// 生成 TSBS DevOps 风格的合成数据点
fn generate_devops_points(device_count: usize, interval_secs: i64, duration_hours: i64) -> Vec<DataPoint> {
    let base_ts: i64 = 1704067200_000_000;
    let points_per_device = (duration_hours * 3600) / interval_secs;
    let mut points = Vec::with_capacity(device_count * points_per_device as usize);

    for device_id in 0..device_count {
        for point_idx in 0..points_per_device {
            let ts = base_ts + point_idx * interval_secs * 1_000_000;
            let mut dp = DataPoint::new("cpu", ts);
            dp.tags.insert("hostname".to_string(), format!("host_{}", device_id));
            dp.tags.insert("region".to_string(), format!("region_{}", device_id % 5));
            dp.tags.insert("datacenter".to_string(), format!("dc_{}", device_id % 3));
            dp.tags.insert("rack".to_string(), format!("rack_{}", device_id % 10));

            // 模拟 9 个 CPU 指标（与 TSBS DevOps 一致）
            dp.fields.insert("usage_user".to_string(), FieldValue::Float((device_id as f64 * 0.3 + (point_idx as f64 * 0.01).sin() * 20.0 + 30.0).min(100.0).max(0.0)));
            dp.fields.insert("usage_system".to_string(), FieldValue::Float((device_id as f64 * 0.1 + (point_idx as f64 * 0.02).cos() * 10.0 + 10.0).min(100.0).max(0.0)));
            dp.fields.insert("usage_idle".to_string(), FieldValue::Float((60.0 + (point_idx as f64 * 0.005).sin() * 15.0).min(100.0).max(0.0)));
            dp.fields.insert("usage_nice".to_string(), FieldValue::Float(device_id as f64 * 0.05 % 5.0));
            dp.fields.insert("usage_iowait".to_string(), FieldValue::Float(device_id as f64 * 0.02 % 3.0));
            dp.fields.insert("usage_steal".to_string(), FieldValue::Float(device_id as f64 * 0.01 % 2.0));
            dp.fields.insert("usage_guest".to_string(), FieldValue::Float(device_id as f64 * 0.005 % 1.0));
            dp.fields.insert("usage_guest_nice".to_string(), FieldValue::Float(0.1));
            dp.fields.insert("count".to_string(), FieldValue::Integer(point_idx as i64));

            points.push(dp);
        }
    }

    points
}

// ==================== 1. 写入吞吐量 ====================
fn bench_write_throughput(engine: &StorageEngine) -> anyhow::Result<()> {
    println!("┌──────────────────────────────────────────────────────────────┐");
    println!("│  1. 写入吞吐量测试 (TSBS DevOps: 100设备 × 1小时 × 10s间隔)  │");
    println!("└──────────────────────────────────────────────────────────────┘\n");

    let points = generate_devops_points(100, 10, 1);
    let total = points.len();
    println!("  数据集: {} 个数据点 ({} 设备 × 9 字段/点)", total, 100);

    // 1a. 单点写入
    let single_count = total.min(1000);
    let start = Instant::now();
    for dp in points.iter().take(single_count) {
        engine.write(dp)?;
    }
    let single_elapsed = start.elapsed();
    let single_rate = single_count as f64 / single_elapsed.as_secs_f64();
    println!("  单点写入: {:.0} points/sec ({} 点, {:.2}ms)", single_rate, single_count, single_elapsed.as_secs_f64() * 1000.0);

    // 1b. 批量写入 (batch=1000)
    let batch_size = 1000;
    let start = Instant::now();
    for chunk in points.chunks(batch_size) {
        engine.write_batch(chunk)?;
    }
    let batch_elapsed = start.elapsed();
    let batch_rate = total as f64 / batch_elapsed.as_secs_f64();
    println!("  批量写入: {:.0} points/sec (batch={}, {:.2}ms)", batch_rate, batch_size, batch_elapsed.as_secs_f64() * 1000.0);

    // 1c. MergedBlock 写入
    let start = Instant::now();
    for dp in &points {
        engine.write_merged(dp)?;
    }
    let merged_elapsed = start.elapsed();
    let merged_rate = total as f64 / merged_elapsed.as_secs_f64();
    println!("  MergedBlock写入: {:.0} points/sec ({:.2}ms)", merged_rate, merged_elapsed.as_secs_f64() * 1000.0);

    // 1d. 大规模写入 (1000设备 × 4小时)
    let large_points = generate_devops_points(1000, 10, 4);
    let large_total = large_points.len();
    println!("\n  大规模测试: {} 设备 × 4小时 = {} 数据点", 1000, large_total);
    let start = Instant::now();
    for chunk in large_points.chunks(batch_size) {
        engine.write_batch(chunk)?;
    }
    let large_elapsed = start.elapsed();
    let large_rate = large_total as f64 / large_elapsed.as_secs_f64();
    println!("  大规模批量写入: {:.0} points/sec ({:.2}s)", large_rate, large_elapsed.as_secs_f64());

    println!();
    Ok(())
}

// ==================== 2. 查询延迟 ====================
fn bench_query_latency(engine: &StorageEngine) -> anyhow::Result<()> {
    println!("┌──────────────────────────────────────────────────────────────┐");
    println!("│  2. 查询延迟测试                                              │");
    println!("└──────────────────────────────────────────────────────────────┘\n");

    let base_ts: i64 = 1704067200_000_000;

    // 2a. 短范围查询 (1分钟)
    let start = Instant::now();
    let results = engine.read_range("cpu", &Tags::new(), base_ts, base_ts + 60_000_000)?;
    let short_time = start.elapsed();
    println!("  短范围查询(1min): {} 结果, {:.2}ms", results.len(), short_time.as_secs_f64() * 1000.0);

    // 2b. 中范围查询 (1小时)
    let start = Instant::now();
    let results = engine.read_range("cpu", &Tags::new(), base_ts, base_ts + 3_600_000_000)?;
    let mid_time = start.elapsed();
    println!("  中范围查询(1h):   {} 结果, {:.2}ms", results.len(), mid_time.as_secs_f64() * 1000.0);

    // 2c. 长范围查询 (4小时)
    let start = Instant::now();
    let results = engine.read_range("cpu", &Tags::new(), base_ts, base_ts + 14_400_000_000)?;
    let long_time = start.elapsed();
    println!("  长范围查询(4h):   {} 结果, {:.2}ms", results.len(), long_time.as_secs_f64() * 1000.0);

    // 2d. SQL 查询
    let query_engine = QueryEngine::new();
    let sql_tests = vec![
        ("简单查询", "SELECT * FROM cpu"),
        ("聚合查询", "SELECT AVG(usage_user) FROM cpu"),
        ("多聚合查询", "SELECT AVG(usage_user), MAX(usage_system), MIN(usage_idle) FROM cpu"),
    ];
    for (name, sql) in sql_tests {
        let start = Instant::now();
        let result = query_engine.execute(sql, engine);
        let elapsed = start.elapsed();
        match result {
            Ok(r) => println!("  SQL {}: {} 列 × {} 行, {:.2}ms", name, r.columns.len(), r.rows.len(), elapsed.as_secs_f64() * 1000.0),
            Err(e) => println!("  SQL {}: 错误 - {}", name, e),
        }
    }

    println!();
    Ok(())
}

// ==================== 3. 压缩效率 ====================
fn bench_compression() -> anyhow::Result<()> {
    println!("┌──────────────────────────────────────────────────────────────┐");
    println!("│  3. 压缩效率测试                                              │");
    println!("└──────────────────────────────────────────────────────────────┘\n");

    let n = 10000;

    // 3a. Delta 时间戳压缩
    let timestamps: Vec<i64> = (0..n).map(|i| 1704067200_000_000 + i as i64 * 10_000_000).collect();
    let raw_ts_size = timestamps.len() * 8;
    let mut delta_enc = DeltaEncoder::new();
    for &ts in &timestamps { delta_enc.encode(ts)?; }
    let compressed_ts = delta_enc.finish();
    let ts_ratio = raw_ts_size as f64 / compressed_ts.len() as f64;
    println!("  Delta时间戳: {}B → {}B (压缩比 {:.1}:1)", raw_ts_size, compressed_ts.len(), ts_ratio);

    // 3b. Gorilla 浮点压缩
    let float_values: Vec<f64> = (0..n).map(|i| 50.0 + (i as f64 * 0.01).sin() * 20.0).collect();
    let raw_float_size = float_values.len() * 8;
    let mut gorilla_enc = GorillaEncoder::new();
    for &v in &float_values { gorilla_enc.encode(v)?; }
    let compressed_float = gorilla_enc.finish();
    let float_ratio = raw_float_size as f64 / compressed_float.len() as f64;
    println!("  Gorilla浮点: {}B → {}B (压缩比 {:.1}:1)", raw_float_size, compressed_float.len(), float_ratio);

    // 3c. 字典编码字符串压缩
    let strings: Vec<&str> = (0..n).map(|i| ["host_0", "host_1", "host_2", "region_us", "region_eu"][i % 5]).collect();
    let raw_str_size: usize = strings.iter().map(|s| s.len()).sum();
    let mut dict_enc = DictionaryEncoder::new();
    for s in &strings { dict_enc.encode(s); }
    let (compressed_str, _) = dict_enc.finish();
    let str_ratio = raw_str_size as f64 / compressed_str.len() as f64;
    println!("  字典编码:    {}B → {}B (压缩比 {:.1}:1)", raw_str_size, compressed_str.len(), str_ratio);

    // 3d. BlockCodec 完整块压缩
    let mut block = DataBlock {
        timestamps: timestamps.clone(),
        fields: std::collections::HashMap::new(),
    };
    block.fields.insert("usage_user".to_string(), float_values.iter().map(|&v| FieldValue::Float(v)).collect());
    block.fields.insert("count".to_string(), (0..n).map(|i| FieldValue::Integer(i as i64)).collect());

    let raw_block_size = block.timestamps.len() * 8 + block.fields.values().map(|v: &Vec<FieldValue>| v.len() * 8).sum::<usize>();
    let codec = BlockCodec;
    let start = Instant::now();
    let compressed_block = codec.compress_block(&block)?;
    let compress_time = start.elapsed();
    let block_ratio = raw_block_size as f64 / compressed_block.timestamps.len() as f64;

    let start = Instant::now();
    let _decompressed = codec.decompress_block(&compressed_block)?;
    let decompress_time = start.elapsed();

    println!("  BlockCodec:  {}B → ~{}B (压缩比 {:.1}:1)", raw_block_size, compressed_block.timestamps.len(), block_ratio);
    println!("  压缩耗时: {:.2}ms, 解压耗时: {:.2}ms", compress_time.as_secs_f64() * 1000.0, decompress_time.as_secs_f64() * 1000.0);

    println!();
    Ok(())
}

// ==================== 4. 向量化 vs 标量 ====================
fn bench_vectorized_vs_scalar(engine: &StorageEngine) -> anyhow::Result<()> {
    println!("┌──────────────────────────────────────────────────────────────┐");
    println!("│  4. 向量化 vs 标量聚合对比                                     │");
    println!("└──────────────────────────────────────────────────────────────┘\n");

    let base_ts: i64 = 1704067200_000_000;
    let results = engine.read_range("cpu", &Tags::new(), base_ts, base_ts + 3_600_000_000)?;
    if results.is_empty() {
        println!("  (无数据，跳过)");
        return Ok(());
    }

    // 标量聚合
    let start = Instant::now();
    let mut sum = 0.0;
    let mut count = 0usize;
    for dp in &results {
        if let Some(FieldValue::Float(v)) = dp.fields.get("usage_user") {
            sum += v;
            count += 1;
        }
    }
    let scalar_avg = if count > 0 { sum / count as f64 } else { 0.0 };
    let scalar_time = start.elapsed();

    // 向量化 SIMD 聚合
    let batch = tsdb_query::vectorized::columnar::ColumnarBatch::from_data_points(&results);
    let start = Instant::now();
    let vec_result = tsdb_query::VectorizedEngine::execute_aggregate(
        &batch, "usage_user", tsdb_query::vectorized::simd_agg::SimdAggFunc::Avg);
    let vec_time = start.elapsed();

    println!("  数据点数: {}", results.len());
    println!("  标量聚合:  avg={:.4}, {:.2}ms", scalar_avg, scalar_time.as_secs_f64() * 1000.0);
    println!("  向量化聚合: avg={:?}, {:.2}ms", vec_result, vec_time.as_secs_f64() * 1000.0);
    if vec_time.as_nanos() > 0 {
        println!("  加速比: {:.1}x", scalar_time.as_secs_f64() / vec_time.as_secs_f64());
    }

    println!();
    Ok(())
}

// ==================== 5. 轻度汇总管道 ====================
fn bench_aggregation_pipeline() -> anyhow::Result<()> {
    println!("┌──────────────────────────────────────────────────────────────┐");
    println!("│  5. 轻度汇总管道性能                                          │");
    println!("└──────────────────────────────────────────────────────────────┘\n");

    let dir = tempfile::TempDir::new()?;
    let store_mgr = Arc::new(AggregationStoreManager::new(dir.path().to_path_buf()));
    let config = PipelineConfig {
        buffer_size: 5000,
        flush_interval_secs: 3600,
        dimensions: vec![TimeDimension::Hour, TimeDimension::Day],
    };
    let pipeline = LightAggregationPipeline::new(config, store_mgr);

    let points = generate_devops_points(100, 10, 1);
    let total = points.len();

    let start = Instant::now();
    for dp in &points {
        pipeline.on_write("benchmark", dp);
    }
    let accumulate_time = start.elapsed();
    let accumulate_rate = total as f64 / accumulate_time.as_secs_f64();

    let start = Instant::now();
    pipeline.flush_all().map_err(|e| anyhow::anyhow!("{}", e))?;
    let flush_time = start.elapsed();

    println!("  数据点: {}", total);
    println!("  累积吞吐: {:.0} points/sec ({:.2}ms)", accumulate_rate, accumulate_time.as_secs_f64() * 1000.0);
    println!("  Flush耗时: {:.2}ms", flush_time.as_secs_f64() * 1000.0);
    println!("  管道总开销: {:.2}%", accumulate_time.as_secs_f64() / (total as f64 / 100000.0) * 100.0);

    println!();
    Ok(())
}

// ==================== 6. 多业务DB隔离 ====================
fn bench_multi_db_isolation() -> anyhow::Result<()> {
    println!("┌──────────────────────────────────────────────────────────────┐");
    println!("│  6. 多业务DB隔离开销                                          │");
    println!("└──────────────────────────────────────────────────────────────┘\n");

    let dir = tempfile::TempDir::new()?;
    let config = CfConfig::default();
    let manager = MultiDbManager::new(dir.path().to_path_buf(), config);

    // 创建 5 个业务数据库
    let businesses = vec!["stocks", "iot", "finance", "orders", "sms"];
    let start = Instant::now();
    for name in &businesses {
        manager.create_database(name)?;
    }
    let create_time = start.elapsed();
    println!("  创建5个DB实例: {:.2}ms", create_time.as_secs_f64() * 1000.0);

    // 跨业务写入
    let points = generate_devops_points(10, 10, 1);
    let start = Instant::now();
    for (i, dp) in points.iter().enumerate() {
        let db = manager.get_database(businesses[i % businesses.len()])?;
        db.write(dp)?;
    }
    let cross_write_time = start.elapsed();
    let cross_rate = points.len() as f64 / cross_write_time.as_secs_f64();
    println!("  跨业务写入: {:.0} points/sec", cross_rate);

    // 单DB写入对比
    let single_db = manager.get_database("stocks")?;
    let start = Instant::now();
    for dp in points.iter().take(100) {
        single_db.write(dp)?;
    }
    let single_write_time = start.elapsed();
    let single_rate = 100.0 / single_write_time.as_secs_f64();
    println!("  单DB写入:   {:.0} points/sec", single_rate);
    println!("  隔离开销:   {:.1}%", (1.0 - cross_rate / single_rate).abs() * 100.0);

    println!();
    Ok(())
}

// ==================== 7. 索引性能 ====================
fn bench_index_performance() -> anyhow::Result<()> {
    println!("┌──────────────────────────────────────────────────────────────┐");
    println!("│  7. 索引性能测试                                              │");
    println!("└──────────────────────────────────────────────────────────────┘\n");

    // 倒排索引
    let mut inv_idx = InvertedIndex::new();
    let n = 10000;

    let start = Instant::now();
    for i in 0..n {
        inv_idx.add_series(i as u64, &[
            ("host".to_string(), format!("host_{}", i % 100)),
            ("region".to_string(), format!("region_{}", i % 5)),
        ]);
    }
    let insert_time = start.elapsed();
    println!("  倒排索引插入: {} 条, {:.2}ms", n, insert_time.as_secs_f64() * 1000.0);

    let start = Instant::now();
    let result = inv_idx.query_exact("host", "host_0");
    let exact_time = start.elapsed();
    println!("  精确查询: {} 匹配, {:.2}μs", result.len(), exact_time.as_secs_f64() * 1_000_000.0);

    let start = Instant::now();
    let result = inv_idx.query_intersection(&[
        ("host".to_string(), "host_0".to_string()),
        ("region".to_string(), "region_0".to_string()),
    ]);
    let intersect_time = start.elapsed();
    println!("  交集查询: {} 匹配, {:.2}μs", result.len(), intersect_time.as_secs_f64() * 1_000_000.0);

    // 序列化/反序列化
    let start = Instant::now();
    let serialized = inv_idx.serialize();
    let ser_time = start.elapsed();
    println!("  序列化: {}B, {:.2}ms", serialized.len(), ser_time.as_secs_f64() * 1000.0);

    let start = Instant::now();
    let _deserialized = InvertedIndex::deserialize(&serialized);
    let deser_time = start.elapsed();
    println!("  反序列化: {:.2}ms", deser_time.as_secs_f64() * 1000.0);

    // IndexManager
    let mut idx_mgr = IndexManager::new();
    let start = Instant::now();
    for i in 0..n {
        let mut tags = BTreeMap::new();
        tags.insert("host".to_string(), format!("host_{}", i % 100));
        idx_mgr.index_data_point("cpu", &tags, 1704067200_000_000 + i as i64 * 10_000_000, i as u64);
    }
    let idx_time = start.elapsed();
    println!("  IndexManager索引: {} 点, {:.2}ms", n, idx_time.as_secs_f64() * 1000.0);

    println!();
    Ok(())
}
