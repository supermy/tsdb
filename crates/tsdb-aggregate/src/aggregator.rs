//! # 聚合引擎 — 多时间维度的数据聚合计算
//!
//! ## 设计目标
//!
//! TSDB 的核心价值之一是 **预聚合（Pre-aggregation）**：
//! 将原始的高频采样数据按不同时间粒度预先汇总，
//! 大幅降低查询时的计算量和数据扫描范围。
//!
//! ## 支持的时间维度
//!
//! | 维度 | 窗口大小 | 典型用途 |
//! |------|---------|----------|
//! | hour | 1 小时 | 实时监控面板 |
//! | day  | 1 天   | 日报/趋势分析 |
//! | week | 7 天   | 周报/环比分析 |
//! | month| 30 天  | 月报/容量规划 |
//!

use std::collections::HashMap;
use tsdb_types::model::DataPoint;

/// 时间维度枚举 — 定义聚合窗口的粒度
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeDimension {
    /// 按小时聚合（3600 秒）
    Hour,
    /// 按天聚合（86400 秒）
    Day,
    /// 按周聚合（604800 秒）
    Week,
    /// 按月聚合（2592000 秒，约 30 天）
    Month,
}

impl TimeDimension {
    /// 根据名称字符串创建 TimeDimension
    ///
    /// # 参数
    /// - `name`: 维度名称（`"hour"`, `"day"`, `"week"`, `"month"`）
    ///
    /// # 返回
    /// 对应的 TimeDimension 枚举值，不匹配则默认为 Day
    pub fn from_name(name: &str) -> Self {
        match name {
            "hour" => TimeDimension::Hour,
            "day" => TimeDimension::Day,
            "week" => TimeDimension::Week,
            "month" => TimeDimension::Month,
            _ => TimeDimension::Day,
        }
    }

    /// 返回该维度对应的时间窗口大小（秒）
    pub fn interval_secs(&self) -> u64 {
        match self {
            TimeDimension::Hour => 3600,
            TimeDimension::Day => 86400,
            TimeDimension::Week => 604800,
            TimeDimension::Month => 2_592_000,
        }
    }

    /// 返回维度名称字符串
    pub fn name(&self) -> &'static str {
        match self {
            TimeDimension::Hour => "hour",
            TimeDimension::Day => "day",
            TimeDimension::Week => "week",
            TimeDimension::Month => "month",
        }
    }

    /// 将微秒级时间戳对齐到该维度的窗口起始位置
    ///
    /// 例如 Day 维度下，`1713158400` (某日 12:00) → `1713110400` (该日 00:00)
    pub fn align_timestamp(&self, timestamp_us: i64) -> i64 {
        let secs = timestamp_us / 1_000_000;
        let interval = self.interval_secs() as i64;
        (secs / interval) * interval * 1_000_000
    }
}

/// 聚合结果 — 单个时间窗口的汇总数据
#[derive(Debug, Clone)]
pub struct AggregationResult {
    /// 目标 measurement 名称
    pub measurement: String,
    /// 聚合的时间维度
    pub dimension: TimeDimension,
    /// 窗口起始时间戳（微秒，已对齐到维度边界）
    pub window_start: i64,
    /// 字段名 → 聚合值的映射（如 `{"avg_usage": 78.5, "max_usage": 99.2}`）
    pub values: HashMap<String, f64>,
}

/// 聚合器 — 执行多维度批量聚合的核心组件
///
/// 接收原始 DataPoint 流，按 (measurement, dimension, window_start) 分桶后
/// 应用指定的聚合函数（SUM/AVG/MIN/MAX/COUNT）。
///
/// ## 使用模式
///
/// 创建 Aggregator 后逐个调用 accumulate() 累加数据点，
/// 最后调用 finalize() 按维度输出聚合结果。
///
pub struct Aggregator {
    /// 内部分桶：(measurement, dimension, window_start) → (字段值累加器, 计数)
    buckets: HashMap<String, (HashMap<String, f64>, usize)>,
}

impl Default for Aggregator {
    fn default() -> Self {
        Self::new()
    }
}

impl Aggregator {
    pub fn new() -> Self {
        Self {
            buckets: HashMap::new(),
        }
    }

    /// 将单个数据点累加到对应的聚合分桶中
    ///
    /// 处理步骤：
    /// 1. 根据 measurement 和各维度生成唯一的 bucket key
    /// 2. 将 float 字段的值加到对应分桶的累加器中
    /// 3. 递增该分桶的数据点计数
    ///
    /// # 参数
    /// - `dp`: 待聚合的原始数据点
    pub fn accumulate(&mut self, dp: &DataPoint) {
        for &dim in &[
            TimeDimension::Hour,
            TimeDimension::Day,
            TimeDimension::Week,
            TimeDimension::Month,
        ] {
            let window_start = dim.align_timestamp(dp.timestamp);
            let bucket_key = format!("{}:{}:{}", dp.measurement, dim.name(), window_start);

            let entry = self
                .buckets
                .entry(bucket_key)
                .or_insert_with(|| (HashMap::new(), 0));

            for (field_name, field_value) in &dp.fields {
                if let Some(f64_val) = field_value.as_f64() {
                    *entry.0.entry(field_name.clone()).or_insert(0.0) += f64_val;
                }
            }
            entry.1 += 1;
        }
    }

    /// 计算指定 measurement 和维度的最终聚合结果
    ///
    /// 遍历所有匹配的分桶，将 SUM 累加值转换为 AVG（除以计数），
    /// 并输出结构化的 AggregationResult 列表。
    ///
    /// # 参数
    /// - `measurement`: 目标指标名称
    /// - `dimension`: 目标时间维度
    ///
    /// # 返回
    /// 该维度下的所有聚合窗口结果列表（按 window_start 升序排列）
    pub fn finalize(
        &mut self,
        measurement: &str,
        dimension: TimeDimension,
    ) -> Vec<AggregationResult> {
        let prefix = format!("{}:{}:", measurement, dimension.name());
        let mut results: Vec<AggregationResult> = Vec::new();

        let keys: Vec<String> = self
            .buckets
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();

        for key in &keys {
            if let Some((values, count)) = self.buckets.remove(key) {
                if count == 0 {
                    continue;
                }
                let parts: Vec<&str> = key.rsplitn(2, ':').collect();
                let window_start = parts[0].parse().unwrap_or(0);

                let avg_values: HashMap<String, f64> = values
                    .into_iter()
                    .map(|(k, v)| (k, v / count as f64))
                    .collect();

                results.push(AggregationResult {
                    measurement: measurement.to_string(),
                    dimension,
                    window_start,
                    values: avg_values,
                });
            }
        }

        results.sort_by_key(|r| r.window_start);
        results
    }

    /// 清空所有内部状态（用于复用同一实例处理新批次数据）
    pub fn measurement_names(&self, dimension: TimeDimension) -> Vec<String> {
        let dim_suffix = format!(":{}", dimension.name());
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for key in self.buckets.keys() {
            if key.contains(&dim_suffix) {
                if let Some(measurement) = key.split(':').next() {
                    if seen.insert(measurement.to_string()) {
                        result.push(measurement.to_string());
                    }
                }
            }
        }
        result
    }

    pub fn reset(&mut self) {
        self.buckets.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_dimension_align() {
        assert_eq!(
            TimeDimension::Day.align_timestamp(1_713_158_400_000_000),
            1_713_139_200_000_000
        );
    }

    #[test]
    fn test_aggregator_accumulate_and_finalize() {
        let mut agg = Aggregator::new();

        let mut dp1 = DataPoint::new("cpu", 1_713_158_400_000_000);
        dp1.fields.insert(
            "usage".to_string(),
            tsdb_types::model::FieldValue::Float(10.0),
        );
        agg.accumulate(&dp1);

        let mut dp2 = DataPoint::new("cpu", 1_713_158_500_000_000);
        dp2.fields.insert(
            "usage".to_string(),
            tsdb_types::model::FieldValue::Float(20.0),
        );
        agg.accumulate(&dp2);

        let results = agg.finalize("cpu", TimeDimension::Day);
        assert_eq!(results.len(), 1);
        assert!((results[0].values.get("usage").unwrap_or(&0.0) - 15.0).abs() < 0.001);
    }
}
