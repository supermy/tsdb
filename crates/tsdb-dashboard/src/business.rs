//! # 业务仪表盘 — 业务指标的可视化展示
//!
//! ## 功能概述
//!
//! BusinessDashboard 从查询结果中提取业务指标，计算趋势方向（上升/下降/稳定），
//! 并渲染为交互式 HTML 页面，供运营人员快速了解业务健康状态。
//!
//! ## 指标卡片展示
//!
//! ```text
//! ┌─────────────────────┐
//! │  CPU USAGE          │  ← 指标名称
//! │  78.50 ↑ +12.3%    │  ← 当前值 + 趋势箭头 + 变化百分比
//! └─────────────────────┘
//! ```
//!

use serde::{Deserialize, Serialize};
use tsdb_types::model::FieldValue;

/// 趋势方向枚举
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Trend {
    /// 上升趋势（变化率 > +5%）
    Up,
    /// 下降趋势（变化率 < -5%）
    Down,
    /// 稳定状态（变化率在 ±5% 以内）
    Stable,
}

/// 单个业务指标 — 仪表盘中的一个数据卡片
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusinessMetric {
    /// 指标名称（如 `"cpu_usage"`, `"order_count"`）
    pub name: String,
    /// 当前值（最新一个数据点的值）
    pub current: f64,
    /// 上一个值（用于计算变化率）
    pub previous: f64,
    /// 单位字符串（如 `"%"`, `"ops/s"`, `"MB"`）
    pub unit: String,
    /// 趋势方向
    pub trend: Trend,
}

/// 业务仪表盘 — 业务指标的聚合与可视化容器
///
/// 支持两种数据源：
/// - **DataPoint 列表**：直接从原始数据点提取字段值
/// - **查询结果集**：从 SQL 查询的 (columns, rows) 中提取
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusinessDashboard {
    /// 业务指标列表（每个 float 字段生成一个指标卡片）
    pub metrics: Vec<BusinessMetric>,
    /// 数据点总数（显示在统计栏中）
    pub total_points: usize,
    /// 出现过的 measurement 名称列表
    pub measurements: Vec<String>,
}

impl BusinessDashboard {
    /// 创建新的空业务仪表盘
    pub fn new() -> Self {
        Self { metrics: Vec::new(), total_points: 0, measurements: Vec::new() }
    }

    /// 从原始 DataPoint 列表构建业务仪表盘
    ///
    /// 处理逻辑：
    /// 1. 遍历所有数据点，按字段名分组收集 float 值
    /// 2. 对每个有 ≥2 个值的字段，取最后两个值计算变化率和趋势
    /// 3. 趋势判定规则：> +5% → Up, < -5% → Down, 否则 → Stable
    ///
    /// # 参数
    /// - `data_points`: 原始 DataPoint 向量引用
    ///
    /// # 返回
    /// 构建完成的 BusinessDashboard 实例
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
                // 计算环比变化率
                let change = if prev > 0.0 { (current - prev) / prev } else { 0.0 };
                let trend = if change > 0.05 { Trend::Up } else if change < -0.05 { Trend::Down } else { Trend::Stable };

                metrics.push(BusinessMetric {
                    name: field_name.clone(), current, previous: prev,
                    unit: "value".to_string(), trend,
                });
            }
        }

        BusinessDashboard { metrics, total_points: data_points.len(), measurements: Self::extract_measurements(data_points) }
    }

    /// 从 SQL 查询结果集构建业务仪表盘
    ///
    /// 与 `from_data_points` 类似，但数据来源是 QueryEngine 的输出格式：
    /// `(columns: &[String], rows: &[Vec<FieldValue>])`
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
                    name: col_name.clone(), current, previous: prev,
                    unit: "value".to_string(), trend,
                });
            }
        }

        BusinessDashboard { metrics, total_points: rows.len(), measurements: columns.to_vec() }
    }

    /// 从 DataPoint 列表中提取去重的 measurement 名称集合
    fn extract_measurements(dps: &[tsdb_types::model::DataPoint]) -> Vec<String> {
        dps.iter().map(|dp| dp.measurement.clone()).collect::<std::collections::HashSet<_>>().into_iter().collect()
    }

    /// 将仪表盘数据导出为 JSON 格式（用于 API 响应或前端消费）
    ///
    /// 输出包含 type、total_data_points、metrics 等字段的 JSON 对象。
    pub fn summary_json(&self) -> serde_json::Value {
        let mut cards = Vec::new();
        for m in &self.metrics {
            let change_pct = if m.previous > 0.0 { (m.current - m.previous) / m.previous * 100.0 } else { 0.0 };
            cards.push(serde_json::json!({
                "name": m.name, "value": m.current, "previous": m.previous,
                "change_pct": format!("{:.1}%", change_pct),
                "trend": match m.trend { Trend::Up => "up", Trend::Down => "down", Trend::Stable => "stable" },
            }));
        }
        serde_json::json!({ "type": "business_dashboard", "total_data_points": self.total_points, "measurements_count": self.measurements.len(), "metrics": cards })
    }

    /// 生成推荐的图表配置（用于关联图表展示）
    pub fn to_chart_config(&self) -> tsdb_chart::ChartConfig {
        tsdb_chart::ChartConfig {
            title: "Business Metrics".to_string(),
            chart_type: tsdb_chart::ChartType::Bar,
            show_legend: true, show_grid: true,
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
