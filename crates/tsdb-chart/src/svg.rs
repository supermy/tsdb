//! # SVG 渲染器 — 将 TimeSeriesChart 渲染为矢量图形
//!
//! ## 设计目标
//!
//! 纯 Rust 实现的 SVG 生成器，无需任何外部 JavaScript 库。
//! 输入 TimeSeriesChart（数据 + 配置），输出 SVG 字符串。
//!
//! ## 支持的图表元素
//!
//! - 坐标轴（X/Y 轴线 + 刻度标签）
//! - 网格线（水平 + 垂直）
//! - 数据可视化（折线/面积/柱状）
//! - 图例（右上角浮动框）
//! - 标题（顶部居中）
//!

use crate::chart::{TimeSeriesChart, ChartType};
use crate::series::TimeSeries;

/// SVG 渲染器 — 将 TimeSeriesChart 转换为 SVG 字符串
///
/// 采用纯字符串拼接方式生成 SVG XML，无外部依赖。
/// 输出的 SVG 可直接嵌入 HTML 页面或作为 `<img>` src 使用。
pub struct SvgRenderer;

impl SvgRenderer {
    /// 将图表渲染为完整的 SVG 文档字符串
    ///
    /// ## 渲染流程
    ///
    /// 1. 计算**绘图区域**尺寸（扣除边距后的有效区域）
    /// 2. 通过 `data_bounds()` 获取数据范围，建立 **数据→像素坐标映射**
    /// 3. 绘制背景、标题、网格线和坐标轴
    /// 4. 遍历每个序列，根据 chart_type 选择绘制策略：
    ///    - **Line**: `<path d="M... L...">` 折线路径
    ///    - **Area**: `<path d="M... L... Z">` 闭合填充路径
    ///    - **Bar**: `<rect>` 矩形柱
    /// 5. 可选：绘制数据点标记和图例
    ///
    /// # 参数
    /// - `chart`: 待渲染的 TimeSeriesChart 对象
    ///
    /// # 返回
    /// 完整的 SVG XML 字符串（以 `<svg>` 开头、`</svg>` 结尾）
    pub fn render(chart: &TimeSeriesChart) -> String {
        let config = &chart.config;
        let w = config.width;
        let h = config.height;
        let m = &config.margin;

        // 计算绘图区域实际可用空间
        let plot_w = w - m.left - m.right;
        let plot_h = h - m.top - m.bottom;

        // 获取数据范围用于坐标映射
        let (min_ts, max_ts, min_val, max_val) = chart.data_bounds();
        let ts_range = max_ts - min_ts;
        let val_range = max_val - min_val;

        // 预分配 8KB 缓冲区减少内存重分配
        let mut svg = String::with_capacity(8192);

        // === SVG 根元素 ===
        svg.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
            w, h, w, h
        ));
        svg.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>");

        // === 标题 ===
        if !config.title.is_empty() {
            svg.push_str(&format!(
                "<text x=\"{}\" y=\"20\" text-anchor=\"middle\" font-family=\"sans-serif\" font-size=\"16\" font-weight=\"bold\">{}</text>",
                w / 2, config.title
            ));
        }

        // === 网格线 + Y 轴刻度标签 ===
        if config.show_grid {
            let grid_lines = 5;
            for i in 0..=grid_lines {
                let y = m.top + (plot_h as f64 * i as f64 / grid_lines as f64) as u32;
                // 水平网格线
                svg.push_str(&format!(
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#e0e0e0\" stroke-width=\"1\"/>",
                    m.left, y, m.left + plot_w, y
                ));
                // Y 轴刻度值
                let val = max_val - val_range * i as f64 / grid_lines as f64;
                svg.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" text-anchor=\"end\" font-family=\"sans-serif\" font-size=\"10\" fill=\"#666\">{:.1}</text>",
                    m.left - 5, y + 4, val
                ));
            }

            // X 轴垂直网格线 + 时间标签
            for i in 0..=5 {
                let x = m.left + (plot_w as f64 * i as f64 / 5.0) as u32;
                svg.push_str(&format!(
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#e0e0e0\" stroke-width=\"1\"/>",
                    x, m.top, x, m.top + plot_h
                ));
                if ts_range > 0.0 {
                    let ts = min_ts + ts_range * i as f64 / 5.0;
                    let dt = chrono::DateTime::from_timestamp(ts as i64 / 1_000_000, 0);
                    let label = dt.map(|d| d.format("%H:%M").to_string()).unwrap_or_default();
                    svg.push_str(&format!(
                        "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-family=\"sans-serif\" font-size=\"10\" fill=\"#666\">{}</text>",
                        x, m.top + plot_h + 15, label
                    ));
                }
            }
        }

        // === 坐标轴线 ===
        svg.push_str(&format!(
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#333\" stroke-width=\"1\"/>",
            m.left, m.top, m.left, m.top + plot_h
        )); // Y 轴
        svg.push_str(&format!(
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#333\" stroke-width=\"1\"/>",
            m.left, m.top + plot_h, m.left + plot_w, m.top + plot_h
        )); // X 轴

        // === 数据序列渲染 ===
        for (idx, series) in chart.series.iter().enumerate() {
            if series.is_empty() { continue; }
            let default_color = "#4e79a7".to_string();
            let color = config.colors.get(idx % config.colors.len()).unwrap_or(&default_color);

            // 将数据点从数据坐标映射到屏幕像素坐标
            let points: Vec<(f64, f64)> = series.timestamps.iter().zip(series.values.iter())
                .map(|(&ts, &v)| {
                    let x = if ts_range > 0.0 {
                        m.left as f64 + ((ts as f64 - min_ts) / ts_range) * plot_w as f64
                    } else { m.left as f64 + plot_w as f64 / 2.0 };
                    let y = if val_range > 0.0 {
                        m.top as f64 + (1.0 - (v - min_val) / val_range) * plot_h as f64
                    } else { m.top as f64 + plot_h as f64 / 2.0 };
                    (x, y)
                }).collect();

            // 根据图表类型选择不同的 SVG 元素
            match config.chart_type {
                ChartType::Area => {
                    if !points.is_empty() {
                        // 面积图：路径 + 闭合到底边形成填充区域
                        let mut d = format!("M {} {}", points[0].0, points[0].1);
                        for p in &points[1..] { d.push_str(&format!(" L {} {}", p.0, p.1)); }
                        d.push_str(&format!(" L {} {} L {} {} Z",
                            points.last().unwrap().0, m.top + plot_h,
                            points[0].0, m.top + plot_h));
                        svg.push_str(&format!(
                            "<path d=\"{}\" fill=\"{}\" fill-opacity=\"0.3\" stroke=\"{}\" stroke-width=\"1.5\"/>",
                            d, color, color
                        ));
                    }
                }
                ChartType::Bar => {
                    if !points.is_empty() {
                        let bar_width = (plot_w as f64 / points.len() as f64 * 0.8).max(1.0);
                        for p in &points {
                            svg.push_str(&format!(
                                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\" rx=\"1\"/>",
                                p.0 - bar_width / 2.0, p.1, bar_width, (m.top + plot_h) as f64 - p.1, color
                            ));
                        }
                    }
                }
                _ => {
                    // 默认折线图
                    if !points.is_empty() {
                        let mut d = format!("M {} {}", points[0].0, points[0].1);
                        for p in &points[1..] { d.push_str(&format!(" L {} {}", p.0, p.1)); }
                        svg.push_str(&format!(
                            "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.5\"/>", d, color
                        ));
                    }
                }
            }

            // 可选：在折线上绘制圆点标记
            if config.show_points && !points.is_empty() {
                for p in &points {
                    svg.push_str(&format!(
                        "<circle cx=\"{}\" cy=\"{}\" r=\"3\" fill=\"{}\"/>", p.0, p.1, color
                    ));
                }
            }
        }

        // === 图例 ===
        if config.show_legend && !chart.series.is_empty() {
            let legend_x = m.left + plot_w - 100;
            let legend_y = m.top + 10;
            svg.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"95\" height=\"{}\" fill=\"white\" stroke=\"#ccc\" rx=\"3\"/>",
                legend_x, legend_y, chart.series.len() as u32 * 20 + 10
            ));
            for (idx, series) in chart.series.iter().enumerate() {
                let default_color2 = "#4e79a7".to_string();
                let color = config.colors.get(idx % config.colors.len()).unwrap_or(&default_color2);
                let y = legend_y + 15 + idx as u32 * 20;
                svg.push_str(&format!(
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"2\"/>",
                    legend_x + 5, y, legend_x + 20, y, color
                ));
                svg.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" font-family=\"sans-serif\" font-size=\"11\" fill=\"#333\">{}</text>",
                    legend_x + 25, y + 4, series.name
                ));
            }
        }

        svg.push_str("</svg>");
        svg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::{ChartConfig, ChartType, TimeSeriesChart};

    #[test]
    fn test_svg_line_chart() {
        let mut chart = TimeSeriesChart::new(ChartConfig { title: "CPU Usage".to_string(), chart_type: ChartType::Line, ..Default::default() });
        let mut series = TimeSeries::new("cpu");
        series.add_point(1_000_000_000, 0.5);
        series.add_point(1_030_000_000, 0.7);
        series.add_point(1_060_000_000, 0.9);
        chart.add_series(series);

        let svg = SvgRenderer::render(&chart);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("CPU Usage"));
        assert!(svg.contains("</svg>"));
    }

    #[test]
    fn test_svg_area_chart() {
        let mut chart = TimeSeriesChart::new(ChartConfig { title: "Memory".to_string(), chart_type: ChartType::Area, ..Default::default() });
        let mut series = TimeSeries::new("mem");
        series.add_point(1_000_000_000, 40.0);
        series.add_point(1_030_000_000, 60.0);
        chart.add_series(series);

        let svg = SvgRenderer::render(&chart);
        assert!(svg.contains("fill-opacity"));
    }

    #[test]
    fn test_svg_bar_chart() {
        let mut chart = TimeSeriesChart::new(ChartConfig { title: "Requests".to_string(), chart_type: ChartType::Bar, ..Default::default() });
        let mut series = TimeSeries::new("req");
        series.add_point(1_000_000_000, 100.0);
        series.add_point(1_030_000_000, 200.0);
        series.add_point(1_060_000_000, 150.0);
        chart.add_series(series);

        let svg = SvgRenderer::render(&chart);
        assert!(svg.contains("<rect"));
    }

    #[test]
    fn test_chart_to_json() {
        let mut chart = TimeSeriesChart::new(ChartConfig::default());
        let mut series = TimeSeries::new("cpu");
        series.add_point(1_000_000_000, 0.5);
        chart.add_series(series);

        let json = chart.to_json();
        assert!(json.contains("cpu"));
        assert!(json.contains("points"));
    }
}
