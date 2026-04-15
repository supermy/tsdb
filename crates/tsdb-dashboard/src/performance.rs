use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetric {
    pub name: String,
    pub value: f64,
    pub max: f64,
    pub unit: String,
    pub level: Level,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Level {
    Good,
    Warning,
    Critical,
}

impl PerformanceMetric {
    pub fn gauge(name: impl Into<String>, value: f64, max: f64, unit: impl Into<String>) -> Self {
        let ratio = if max > 0.0 { value / max } else { 1.0 };
        let level = if ratio < 0.7 { Level::Good } else if ratio < 0.9 { Level::Warning } else { Level::Critical };
        Self {
            name: name.into(),
            value,
            max,
            unit: unit.into(),
            level,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub cpu_usage: f64,
    pub memory_usage_mb: f64,
    pub disk_usage_gb: f64,
    pub active_connections: u64,
    pub write_ops_per_sec: f64,
    pub read_ops_per_sec: f64,
    pub query_latency_ms: f64,
    pub compression_ratio: f64,
}

pub struct PerformanceDashboard {
    pub system_metrics: Option<SystemMetrics>,
    pub gauges: Vec<PerformanceMetric>,
    pub history: Vec<TimestampRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimestampRecord {
    pub timestamp: i64,
    pub writes: u64,
    pub reads: u64,
    pub bytes_written: u64,
    pub bytes_read: u64,
}

impl PerformanceDashboard {
    pub fn new() -> Self {
        Self {
            system_metrics: None,
            gauges: Vec::new(),
            history: Vec::new(),
        }
    }

    pub fn with_system_metrics(mut self, sys: SystemMetrics) -> Self {
        self.system_metrics = Some(sys);
        self
    }

    pub fn add_gauge(&mut self, metric: PerformanceMetric) {
        self.gauges.push(metric);
    }

    pub fn record(&mut self, record: TimestampRecord) {
        self.history.push(record);
        if self.history.len() > 3600 {
            self.history.remove(0);
        }
    }

    pub fn summary_json(&self) -> serde_json::Value {
        let mut gauges_json = Vec::new();
        for g in &self.gauges {
            gauges_json.push(serde_json::json!({
                "name": g.name,
                "value": g.value,
                "max": g.max,
                "unit": g.unit,
                "level": match g.level {
                    Level::Good => "good",
                    Level::Warning => "warning",
                    Level::Critical => "critical",
                },
                "percentage": if g.max > 0.0 { (g.value / g.max * 100.0) as i64 } else { 0 },
            }));
        }

        let sys_json = if let Some(sys) = &self.system_metrics {
            Some(serde_json::json!({
                "cpu_usage": sys.cpu_usage,
                "memory_usage_mb": sys.memory_usage_mb,
                "disk_usage_gb": sys.disk_usage_gb,
                "active_connections": sys.active_connections,
                "write_ops_per_sec": sys.write_ops_per_sec,
                "read_ops_per_sec": sys.read_ops_per_sec,
                "query_latency_ms": sys.query_latency_ms,
                "compression_ratio": sys.compression_ratio,
            }))
        } else {
            None
        };

        serde_json::json!({
            "type": "performance_dashboard",
            "gauges": gauges_json,
            "system": sys_json,
            "history_records": self.history.len(),
        })
    }

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
        let good = PerformanceMetric::gauge("test_good", 30.0, 100.0, "%");
        assert_eq!(good.level, Level::Good);

        let warn = PerformanceMetric::gauge("test_warn", 80.0, 100.0, "%");
        assert_eq!(warn.level, Level::Warning);

        let crit = PerformanceMetric::gauge("test_crit", 95.0, 100.0, "%");
        assert_eq!(crit.level, Level::Critical);
    }

    #[test]
    fn test_performance_summary_json() {
        let mut dash = PerformanceDashboard::new();
        dash.add_gauge(PerformanceMetric::gauge("cpu", 50.0, 100.0, "%"));
        dash.record(TimestampRecord {
            timestamp: chrono::Utc::now().timestamp_micros(),
            writes: 1000,
            reads: 2000,
            bytes_written: 1024 * 1024,
            bytes_read: 2048 * 1024,
        });

        let json = dash.summary_json();
        assert_eq!(json["type"], "performance_dashboard");
        assert_eq!(json["history_records"], 1);
    }
}
