//! # 图表配置与容器 — 图表外观和数据的统一抽象
//!
//! ## 图表类型
//!
//! | 类型 | ChartType 枚举值 | 适用场景 |
//! |------|-----------------|----------|
//! | 折线图 | Line | 趋势分析、时序对比 |
//! | 面积图 | Area | 占比展示、累积量 |
//! | 柱状图 | Bar | 分类对比、离散数据 |
//! | 散点图 | Scatter | 相关性分析 |
//!

use crate::series::TimeSeries;
use serde::{Deserialize, Serialize};

/// 图表类型枚举
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ChartType {
    /// 折线图 — 用折线连接各数据点
    Line,
    /// 面积图 — 折线下方填充半透明色
    Area,
    /// 柱状图 — 用矩形柱表示每个数据点
    Bar,
    /// 散点图 — 仅绘制数据点（预留）
    Scatter,
}

/// 图表边距配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Margin {
    /// 上边距（像素，用于标题等）
    pub top: u32,
    /// 右边距（像素，用于图例等）
    pub right: u32,
    /// 下边距（像素，用于 X 轴标签）
    pub bottom: u32,
    /// 左边距（像素，用于 Y 轴标签）
    pub left: u32,
}

/// 图表全局配置 — 控制图表的外观、尺寸和显示选项
///
/// 支持从 JSON/YAML 反序列化，便于前端动态调整。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartConfig {
    /// 图表标题（显示在顶部居中位置）
    pub title: String,
    /// 画布宽度（像素），默认 800
    pub width: u32,
    /// 画布高度（像素），默认 400
    pub height: u32,
    /// 图表类型（Line/Area/Bar/Scatter）
    pub chart_type: ChartType,
    /// X 轴标签文字，默认 "Time"
    pub x_label: String,
    /// Y 轴标签文字，默认 "Value"
    pub y_label: String,
    /// 是否显示图例（默认 true）
    pub show_legend: bool,
    /// 是否显示网格线（默认 true）
    pub show_grid: bool,
    /// 是否在折线上绘制数据点标记（默认 false）
    pub show_points: bool,
    /// 四周边距设置
    pub margin: Margin,
    /// 各序列的颜色列表（循环使用）
    pub colors: Vec<String>,
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
            margin: Margin { top: 30, right: 30, bottom: 50, left: 60 },
            // Tableau 10 配色方案（专业、易区分）
            colors: vec![
                "#4e79a7".to_string(), "#f28e2b".to_string(), "#e15759".to_string(),
                "#76b7b2".to_string(), "#59a14f".to_string(), "#edc948".to_string(),
                "#b07aa1".to_string(), "#ff9da7".to_string(),
            ],
        }
    }
}

/// 时间序列图表 — 配置 + 数据的完整图表对象
///
/// 是 SVG 渲染器的输入数据结构。包含：
/// - `config`: 图表的外观配置
/// - `series`: 一个或多个 TimeSeries 数据序列
pub struct TimeSeriesChart {
    /// 图表配置（尺寸、颜色、样式等）
    pub config: ChartConfig,
    /// 数据序列列表（支持多条线叠加显示）
    pub series: Vec<TimeSeries>,
}

impl TimeSeriesChart {
    /// 创建新的空图表实例
    ///
    /// # 参数
    /// - `config`: 图表配置（可使用 `ChartConfig::default()` 快速创建）
    pub fn new(config: ChartConfig) -> Self {
        Self { config, series: Vec::new() }
    }

    /// 向图表中添加一个时间序列
    ///
    /// 可多次调用以叠加显示多个指标。
    pub fn add_series(&mut self, series: TimeSeries) {
        self.series.push(series);
    }

    /// 计算所有序列的数据边界范围
    ///
    /// 返回 `(min_ts, max_ts, min_val, max_val)` 四元组，
    /// 用于坐标轴的自动缩放计算。值域会额外添加 5% 的 padding
    /// 以避免数据点贴着坐标轴边缘。
    ///
    /// # 返回
    /// (最小时间戳, 最大时间戳, 最小值, 最大值)
    pub fn data_bounds(&self) -> (f64, f64, f64, f64) {
        let mut min_ts = f64::INFINITY;
        let mut max_ts = f64::NEG_INFINITY;
        let mut min_val = f64::INFINITY;
        let mut max_val = f64::NEG_INFINITY;

        for s in &self.series {
            if s.is_empty() { continue; }
            let ts_min = *s.timestamps.first().unwrap_or(&0) as f64;
            let ts_max = *s.timestamps.last().unwrap_or(&0) as f64;
            min_ts = min_ts.min(ts_min);
            max_ts = max_ts.max(ts_max);
            min_val = min_val.min(s.min_value());
            max_val = max_val.max(s.max_value());
        }

        // 空数据的兜底值
        if min_ts == f64::INFINITY { min_ts = 0.0; }
        if max_ts == f64::NEG_INFINITY { max_val = 1.0; }
        if min_val == f64::INFINITY { min_val = 0.0; }
        if max_val == f64::NEG_INFINITY { max_val = 1.0; }

        // 值域添加 5% padding
        let val_range = max_val - min_val;
        let val_padding = val_range * 0.05;
        min_val -= val_padding;
        max_val += val_padding;

        (min_ts, max_ts, min_val, max_val)
    }

    /// 将图表数据导出为 JSON 字符串
    ///
    /// 用于 API 响应或前端 JavaScript 库（如 ECharts、Chart.js）消费。
    /// 输出格式：`{ config: {...}, series: [{ name, points }, ...] }`
    pub fn to_json(&self) -> String {
        let data: Vec<serde_json::Value> = self.series.iter().map(|s| {
            let points: Vec<Vec<f64>> = s.timestamps.iter().zip(s.values.iter())
                .map(|(&ts, &v)| vec![ts as f64, v])
                .collect();
            serde_json::json!({ "name": s.name, "points": points })
        }).collect();

        serde_json::json!({ "config": self.config, "series": data }).to_string()
    }
}
