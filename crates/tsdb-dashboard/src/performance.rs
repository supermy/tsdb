//! # 性能仪表盘 — TSDB 运行时性能指标监控
//!
//! ## 监控维度
//!
//! | 指标类别 | 典型指标 | 说明 |
//! |---------|---------|------|
//! | 吞吐量 | write_throughput, read_throughput | ops/s |
//! | 延迟 | query_latency_p99 | ms |
//! | 资源 | disk_utilization, memory_usage | % |
//! | 效率 | compression_ratio | x (倍率) |
//!

use serde::{Deserialize, Serialize};
use std::time::Instant;

/// 性能健康等级
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Level {
    /// 正常（使用率 < 70%）
    Good,
    /// 警告（使用率 70%~90%）
    Warning,
    /// 危险（使用率 > 90%）
    Critical,
}

/// 单个性能指标 — 带有自动等级判定的仪表项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetric {
    /// 指标名称（如 `"write_throughput"`, `"query_latency"`）
    pub name: String,
    /// 当前值
    pub value: f64,
    /// 最大值/阈值（用于计算百分比和等级）
    pub max: f64,
    /// 单位字符串
    pub unit: String,
    /// 自动判定的健康等级
    pub level: Level,
}

impl PerformanceMetric {
    /// 创建带自动等级判定的性能指标
    ///
    /// 等级判定规则：
    /// - `value/max < 0.7` → **Good**（绿色）
    /// - `0.7 ≤ value/max < 0.9` → **Warning**（橙色）
    /// - `value/max ≥ 0.9` → **Critical**（红色）
    ///
    /// # 参数
    /// - `name`: 指标名称
    /// - `value`: 当前测量值
    /// - `max`: 阈值/最大值
    /// - `unit`: 单位字符串
    ///
    /// # 返回
    /// 已判定等级的 PerformanceMetric 实例
    pub fn gauge(name: impl Into<String>, value: f64, max: f64, unit: impl Into<String>) -> Self {
        let ratio = if max > 0.0 { value / max } else { 1.0 };
        let level = if ratio < 0.7 { Level::Good } else if ratio < 0.9 { Level::Warning } else { Level::Critical };
        Self { name: name.into(), value, max, unit: unit.into(), level }
    }
}

/// 系统指标快照 — 一组系统级的性能数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    /// CPU 使用率 (%)
    pub cpu_usage: f64,
    /// 内存使用量 (MB)
    pub memory_usage_mb: f64,
    /// 磁盘使用量 (GB)
    pub disk_usage_gb: f64,
    /// 当前活跃连接数
    pub active_connections: u64,
    /// 写入操作速率 (ops/s)
    pub write_ops_per_sec: f64,
    /// 读取操作速率 (ops/s)
    pub read_ops_per_sec: f64,
    /// 查询延迟 P99 (ms)
    pub query_latency_ms: f64,
    /// 数据压缩比 (原始大小 / 压缩后大小)
    pub compression_ratio: f64,
}

/// 性能仪表盘 — 运行时性能指标的聚合与展示容器
///
/// 包含三类数据：
/// - **gauges**: 实时性能指标卡片（带进度条和颜色编码）
/// - **system_metrics**: 可选的系统级指标快照
/// - **history**: 时间序列历史记录（用于趋势图，最多保留 3600 条）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceDashboard {
    /// 系统指标快照（可选，需要外部采集器填充）
    pub system_metrics: Option<SystemMetrics>,
    /// 性能指标卡片列表
    pub gauges: Vec<PerformanceMetric>,
    /// 历史时间戳记录（环形缓冲区，上限 3600 条 ≈ 1 小时@1Hz）
    pub history: Vec<TimestampRecord>,
}

/// 单条时间戳记录 — 用于写入/读取量的历史追踪
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimestampRecord {
    /// 记录时间戳（微秒）
    pub timestamp: i64,
    /// 该时间窗口内的写入操作数
    pub writes: u64,
    /// 该时间窗口内的读取操作数
    pub reads: u64,
    /// 写入字节数
    pub bytes_written: u64,
    /// 读取字节数
    pub bytes_read: u64,
}

impl PerformanceDashboard {
    /// 创建新的空性能仪表盘
    pub fn new() -> Self {
        Self { system_metrics: None, gauges: Vec::new(), history: Vec::new() }
    }

    /// 设置系统指标快照（链式调用风格）
    pub fn with_system_metrics(mut self, sys: SystemMetrics) -> Self {
        self.system_metrics = Some(sys);
        self
    }

    /// 添加一个性能指标卡片
    pub fn add_gauge(&mut self, metric: PerformanceMetric) {
        self.gauges.push(metric);
    }

    /// 记录一条时间戳历史数据
    ///
    /// 当历史记录超过 3600 条时自动移除最旧的记录（FIFO）。
    pub fn record(&mut self, record: TimestampRecord) {
        self.history.push(record);
        if self.history.len() > 3600 {
            self.history.remove(0);
        }
    }

    /// 将仪表盘数据导出为 JSON 格式
    ///
    /// 用于 API 响应或前端 JavaScript 渲染。
    pub fn summary_json(&self) -> serde_json::Value {
        let mut gauges_json = Vec::new();
        for g in &self.gauges {
            gauges_json.push(serde_json::json!({
                "name": g.name, "value": g.value, "max": g.max, "unit": g.unit,
                "level": match g.level { Level::Good => "good", Level::Warning => "warning", Level::Critical => "critical" },
                "percentage": if g.max > 0.0 { (g.value / g.max * 100.0) as i64 } else { 0 },
            }));
        }

        let sys_json = if let Some(sys) = &self.system_metrics {
            Some(serde_json::json!({ "cpu_usage": sys.cpu_usage, "memory_usage_mb": sys.memory_usage_mb, "disk_usage_gb": sys.disk_usage_gb, "active_connections": sys.active_connections, "write_ops_per_sec": sys.write_ops_per_sec, "read_ops_per_sec": sys.read_ops_per_sec, "query_latency_ms": sys.query_latency_ms, "compression_ratio": sys.compression_ratio }))
        } else { None };

        serde_json::json!({ "type": "performance_dashboard", "gauges": gauges_json, "system": sys_json, "history_records": self.history.len() })
    }

    /// 生成一组默认的性能指标卡片
    ///
    /// 提供常用的 TSDB 性能基准指标模板：
    /// - 写入吞吐量、读取吞吐量、查询延迟、压缩比、磁盘利用率、内存利用率
    ///
    /// # 参数
    /// - `write_rate`: 写入速率 (ops/s)
    /// - `read_rate`: 读取速率 (ops/s)
    /// - `latency_ms`: 查询 P99 延迟 (ms)
    /// - `compression`: 压缩比 (x)
    ///
    /// # 返回
    /// 预配置的 PerformanceMetric 向量
    pub fn default_gauges(write_rate: f64, read_rate: f64, latency_ms: f64, compression: f64) -> Vec<PerformanceMetric> {
        vec![
            PerformanceMetric::gauge("write_throughput", write_rate, 100_000.0, "ops/s"),
            PerformanceMetric::gauge("read_throughput", read_rate, 500_000.0, "ops/s"),
            PerformanceMetric::gauge("query_latency_p99", latency_ms, 10.0, "ms"),
            PerformanceMetric::gauge("compression_ratio", compression, 20.0, "x"),
            PerformanceMetric::gauge("disk_utilization", 45.0, 100.0, "%"),
            PerformanceMetric::gauge("memory_usage", 60.0, 100.0, "%"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_performance_gauge_levels() {
        // Good: 30% 使用率
        let good = PerformanceMetric::gauge("test_good", 30.0, 100.0, "%");
        assert_eq!(good.level, Level::Good);

        // Warning: 80% 使用率
        let warn = PerformanceMetric::gauge("test_warn", 80.0, 100.0, "%");
        assert_eq!(warn.level, Level::Warning);

        // Critical: 95% 使用率
        let crit = PerformanceMetric::gauge("test_crit", 95.0, 100.0, "%");
        assert_eq!(crit.level, Level::Critical);
    }

    #[test]
    fn test_performance_summary_json() {
        let mut dash = PerformanceDashboard::new();
        dash.add_gauge(PerformanceMetric::gauge("cpu", 50.0, 100.0, "%"));
        dash.record(TimestampRecord {
            timestamp: chrono::Utc::now().timestamp_micros(),
            writes: 1000, reads: 2000,
            bytes_written: 1024 * 1024, bytes_read: 2048 * 1024,
        });

        let json = dash.summary_json();
        assert_eq!(json["type"], "performance_dashboard");
        assert_eq!(json["history_records"], 1);
    }
}
