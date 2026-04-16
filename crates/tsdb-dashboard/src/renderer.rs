//! # 仪表盘渲染器 — 将 Dashboard 数据渲染为 HTML 页面
//!
//! ## 功能
//!
//! DashboardRenderer 提供两个静态方法：
//! - `render_business_html()`: 业务仪表盘 → 响应式 HTML（指标卡片网格）
//! - `render_performance_html()`: 性能仪表盘 → 响应式 HTML（进度条 + 系统概览）
//!
//! ## 输出格式
//!
//! 完整的 HTML5 文档，包含内联 CSS 样式，可直接在浏览器中打开，
//! 无需任何外部依赖（无 CDN、无 JavaScript 框架）。
//!

use crate::business::BusinessDashboard;
use crate::performance::PerformanceDashboard;

/// 仪表盘输出格式枚举
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DashboardFormat {
    /// HTML 格式（完整网页，含 CSS 样式）
    Html,
    /// JSON 格式（结构化数据，供前端框架消费）
    Json,
    /// SVG 格式（矢量图形，用于嵌入或下载）
    Svg,
}

/// 仪表盘渲染器 — Dashboard 数据 → HTML 页面的转换器
///
/// 采用纯字符串拼接生成 HTML，零外部依赖。
/// 样式使用现代 CSS Grid/Flexbox 布局，支持响应式设计。
pub struct DashboardRenderer;

impl DashboardRenderer {
    /// 渲染业务仪表盘为完整的 HTML 页面
    ///
    /// 生成响应式 HTML，包含指标卡片网格和统计信息栏。
    pub fn render_business_html(dash: &BusinessDashboard) -> String {
        let json = dash.summary_json();

        // 从 JSON 中提取各指标数据，生成 HTML 卡片
        let metrics_html = json["metrics"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|m| {
                        let name = m["name"].as_str().unwrap_or("");
                        let value = m["value"].as_f64().unwrap_or(0.0);
                        let change = m["change_pct"].as_str().unwrap_or("");
                        let trend = m["trend"].as_str().unwrap_or("");
                        // 根据趋势选择颜色和箭头符号
                        let color = match trend {
                            "up" => "#e15759",
                            "down" => "#59a14f",
                            _ => "#4e79a7",
                        };
                        let arrow = match trend {
                            "up" => "\u{2191}",
                            "down" => "\u{2193}",
                            _ => "\u{2192}",
                        };
                        format!(
                            r#"<div class="metric-card">
                        <div class="metric-name">{}</div>
                        <div class="metric-value" style="color:{}">{:.2} {}</div>
                        <div class="metric-change">{}</div>
                    </div>"#,
                            name, color, value, arrow, change
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();

        // 组装完整 HTML 页面（含内联 CSS 样式表）
        format!(
            r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>TSDB Business Dashboard</title>
<style>
body {{ font-family: -apple-system, BlinkMacSystemFont, sans-serif; background: #f5f5f5; margin: 0; padding: 20px; }}
.header {{ background: #4e79a7; color: white; padding: 16px 24px; border-radius: 8px; margin-bottom: 20px; }}
.header h1 {{ margin: 0; font-size: 22px; }}
.grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(250px, 1fr)); gap: 16px; }}
.metric-card {{ background: white; border-radius: 8px; padding: 20px; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }}
.metric-name {{ font-size: 13px; color: #666; margin-bottom: 8px; text-transform: uppercase; letter-spacing: 0.5px; }}
.metric-value {{ font-size: 28px; font-weight: bold; margin-bottom: 4px; }}
.metric-change {{ font-size: 12px; color: #888; }}
.stats {{ display: flex; gap: 24px; margin-bottom: 20px; flex-wrap: wrap; }}
.stat-box {{ background: white; padding: 16px 24px; border-radius: 8px; min-width: 150px; }}
.stat-label {{ font-size: 12px; color: #666; }}
.stat-value {{ font-size: 20px; font-weight: bold; color: #333; }}
</style></head><body>
<div class="header"><h1>TSDB Business Dashboard</h1></div>
<div class="stats">
    <div class="stat-box"><div class="stat-label">Total Points</div><div class="stat-value">{}</div></div>
    <div class="stat-box"><div class="stat-label">Measurements</div><div class="stat-value">{}</div></div>
</div>
<div class="grid">{}</div>
</body></html>"#,
            dash.total_points,
            dash.measurements.len(),
            metrics_html
        )
    }

    /// 渲染性能仪表盘为完整的 HTML 页面
    ///
    /// 生成响应式 HTML，包含进度条卡片网格和系统概览区域。
    /// 每个进度条卡片根据等级自动着色：Good→绿色，Warning→橙色，Critical→红色。
    pub fn render_performance_html(dash: &PerformanceDashboard) -> String {
        let json = dash.summary_json();

        // 生成进度条卡片 HTML
        let gauges_html = json["gauges"].as_array().map(|arr| {
            arr.iter().map(|g| {
                let name = g["name"].as_str().unwrap_or("");
                let value = g["value"].as_f64().unwrap_or(0.0);
                let pct = g["percentage"].as_i64().unwrap_or(0);
                let level = g["level"].as_str().unwrap_or("");
                // 根据等级选择颜色方案
                let (color, bg_color) = match level {
                    "good" => ("#59a14f", "#eaf5ea"),
                    "warning" => ("#f28e2b", "#fef3e6"),
                    "critical" => ("#e15759", "#fce8e6"),
                    _ => ("#4e79a7", "#eef4fa"),
                };
                let bar_width = (pct.min(100) as f64 / 100.0 * 200.0).max(1.0);
                format!(
                    r#"<div class="gauge-card">
                        <div class="gauge-name">{}</div>
                        <div class="gauge-bar-bg"><div class="gauge-bar-fill" style="width:{:.0}px;background:{}"></div></div>
                        <div class="gauge-info"><span style="color:{};font-weight:bold;">{:.1}</span> <span class="unit">{}</span> <span class="pct">{}%</span></div>
                    </div>"#,
                    name, bar_width, bg_color, color, value, g["unit"].as_str().unwrap_or(""), pct
                )
            }).collect::<Vec<_>>().join("")
        }).unwrap_or_default();

        format!(
            r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>TSDB Performance Dashboard</title>
<style>
body {{ font-family: -apple-system, BlinkMacSystemFont, sans-serif; background: #f5f5f5; margin: 0; padding: 20px; }}
.header {{ background: #59a14f; color: white; padding: 16px 24px; border-radius: 8px; margin-bottom: 20px; }}
.header h1 {{ margin: 0; font-size: 22px; }}
.gauge-grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 16px; }}
.gauge-card {{ background: white; border-radius: 8px; padding: 20px; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }}
.gauge-name {{ font-size: 13px; color: #666; margin-bottom: 12px; text-transform: uppercase; letter-spacing: 0.5px; }}
.gauge-bar-bg {{ height: 8px; background: #eee; border-radius: 4px; overflow: hidden; margin-bottom: 8px; }}
.gauge-bar-fill {{ height: 100%; border-radius: 4px; transition: width 0.3s ease; }}
.gauge-info {{ display: flex; align-items: center; gap: 8px; font-size: 14px; }}
.unit {{ color: #888; }}
.pct {{ color: #aaa; margin-left: auto; }}
.system-section {{ background: white; border-radius: 8px; padding: 20px; margin-top: 20px; }}
.system-title {{ font-size: 16px; font-weight: bold; margin-bottom: 16px; color: #333; }}
.system-grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 16px; }}
.sys-item {{ padding: 12px; background: #fafafa; border-radius: 6px; }}
.sys-label {{ font-size: 12px; color: #666; margin-bottom: 4px; }}
.sys-value {{ font-size: 18px; font-weight: bold; color: #333; }}
</style></head><body>
<div class="header"><h1>TSDB Performance Dashboard</h1></div>
<div class="gauge-grid">{}</div>
<div class="system-section">
    <div class="system-title">System Overview</div>
    <div class="sys-grid">
        <div class="sys-item"><div class="sys-label">History Records</div><div class="sys-value">{}</div></div>
    </div>
</div>
</body></html>"#,
            gauges_html,
            json["history_records"].as_u64().unwrap_or(0)
        )
    }
}
