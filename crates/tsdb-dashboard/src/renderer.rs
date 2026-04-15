use crate::business::BusinessDashboard;
use crate::performance::PerformanceDashboard;
use tsdb_chart::SvgRenderer;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DashboardFormat {
    Html,
    Json,
    Svg,
}

pub struct DashboardRenderer;

impl DashboardRenderer {
    pub fn render_business_html(dash: &BusinessDashboard) -> String {
        let json = dash.summary_json();
        let metrics_html = json["metrics"].as_array().map(|arr| {
            arr.iter().map(|m| {
                let name = m["name"].as_str().unwrap_or("");
                let value = m["value"].as_f64().unwrap_or(0.0);
                let change = m["change_pct"].as_str().unwrap_or("");
                let trend = m["trend"].as_str().unwrap_or("");
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
            }).collect::<Vec<_>>().join("")
        }).unwrap_or_default();

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
            dash.total_points, dash.measurements.len(), metrics_html
        )
    }

    pub fn render_performance_html(dash: &PerformanceDashboard) -> String {
        let json = dash.summary_json();
        let gauges_html = json["gauges"].as_array().map(|arr| {
            arr.iter().map(|g| {
                let name = g["name"].as_str().unwrap_or("");
                let value = g["value"].as_f64().unwrap_or(0.0);
                let pct = g["percentage"].as_i64().unwrap_or(0);
                let level = g["level"].as_str().unwrap_or("");
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
            gauges_html, json["history_records"].as_u64().unwrap_or(0)
        )
    }
}
