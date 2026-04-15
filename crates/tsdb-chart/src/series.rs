//! # 时间序列数据结构 — 图表数据的基本单元
//!
//! TimeSeries 是图表渲染的最小数据单位，包含：
//! - **name**: 序列名称（如 `"cpu_usage"`, `"memory_used"`）
//! - **timestamps**: 有序时间戳向量（微秒级，单调递增）
//! - **values**: 对应的测量值向量（f64 浮点数）
//!

use tsdb_types::model::Timestamp;

/// 时间序列 — 一组 (时间戳, 值) 数据点的有序集合
///
/// 是 tsdb-chart 模块的核心数据结构，所有图表类型
/// （折线图、面积图、柱状图）都基于一个或多个 TimeSeries 渲染。
#[derive(Debug, Clone)]
pub struct TimeSeries {
    /// 序列名称（用于图例显示和数据标识）
    pub name: String,
    /// 微秒级时间戳向量（必须与 values 等长且单调递增）
    pub timestamps: Vec<Timestamp>,
    /// 对应的测量值向量（f64 类型，支持整数和浮点数）
    pub values: Vec<f64>,
}

impl TimeSeries {
    /// 创建新的空时间序列
    ///
    /// # 参数
    /// - `name`: 序列名称（如 `"cpu"`, `"memory"`）
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), timestamps: Vec::new(), values: Vec::new() }
    }

    /// 向序列末尾追加单个数据点
    ///
    /// # 参数
    /// - `ts`: 微秒级时间戳
    /// - `value`: 测量值
    pub fn add_point(&mut self, ts: Timestamp, value: f64) {
        self.timestamps.push(ts);
        self.values.push(value);
    }

    /// 从 (timestamp, value) 对列表批量创建时间序列
    ///
    /// # 参数
    /// - `name`: 序列名称
    /// - `pairs`: (时间戳, 值) 对的向量
    pub fn from_pairs(name: impl Into<String>, pairs: Vec<(Timestamp, f64)>) -> Self {
        let mut timestamps = Vec::with_capacity(pairs.len());
        let mut values = Vec::with_capacity(pairs.len());
        for (ts, v) in pairs { timestamps.push(ts); values.push(v); }
        Self { name: name.into(), timestamps, values }
    }

    /// 返回序列中的最小值
    pub fn min_value(&self) -> f64 {
        self.values.iter().copied().fold(f64::INFINITY, f64::min)
    }

    /// 返回序列中的最大值
    pub fn max_value(&self) -> f64 {
        self.values.iter().copied().fold(f64::NEG_INFINITY, f64::max)
    }

    /// 返回序列中所有值的算术平均值
    ///
    /// 空序列返回 0.0。
    pub fn avg_value(&self) -> f64 {
        if self.values.is_empty() { return 0.0; }
        self.values.iter().sum::<f64>() / self.values.len() as f64
    }

    /// 返回序列中的数据点数量
    pub fn len(&self) -> usize { self.values.len() }

    /// 判断序列是否为空
    pub fn is_empty(&self) -> bool { self.values.is_empty() }

    /// 下采样 — 将数据点数量缩减到指定上限
    ///
    /// 当原始点数超过 `max_points` 时，采用**分段均值聚合**策略：
    /// 将连续的点分为 N 个桶，每个桶内取平均值作为代表点。
    ///
    /// ## 适用场景
    ///
    /// 当查询返回数百万个数据点时，直接渲染会导致：
    /// - SVG 文件过大（>10MB）
    /// - 浏览器渲染卡顿
    /// - 网络传输慢
    ///
    /// 通过下采样到 ~1000 个点可保持视觉精度同时大幅减少开销。
    ///
    /// # 参数
    /// - `max_points`: 允许的最大数据点数量
    ///
    /// # 返回
    /// 下采样后的新 TimeSeries（原序列不变）
    pub fn downsample(&self, max_points: usize) -> TimeSeries {
        if self.len() <= max_points { return self.clone(); }

        let step = self.len() as f64 / max_points as f64;
        let mut new_ts = Vec::with_capacity(max_points);
        let mut new_vals = Vec::with_capacity(max_points);

        for i in 0..max_points {
            let start = (i as f64 * step) as usize;
            let end = ((i + 1) as f64 * step).min(self.len() as f64) as usize;
            if start >= end { continue; }

            // 取桶内均值作为代表值
            let avg: f64 = self.values[start..end].iter().sum::<f64>() / (end - start) as f64;
            let mid_ts = self.timestamps[(start + end) / 2];
            new_ts.push(mid_ts);
            new_vals.push(avg);
        }

        TimeSeries { name: self.name.clone(), timestamps: new_ts, values: new_vals }
    }
}
