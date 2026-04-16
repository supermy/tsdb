//! # HTTP RESTful API 服务
//!
//! 提供 RESTful 端点，支持多业务数据库隔离。

use tsdb_core::storage::StorageEngine;
use tsdb_core::storage::multi_db::MultiDbManager;
use tsdb_core::error::{TsdbError, Result};
use tsdb_query::QueryEngine;
use tsdb_types::model::{DataPoint, FieldValue};
use std::sync::Arc;
use std::io::{Read, Write};
use tracing::{info, error};

/// 启动 HTTP API 服务
pub fn start_http_server(addr: &str, db: Arc<StorageEngine>, query_engine: QueryEngine, db_manager: Arc<MultiDbManager>) {
    let listener = match std::net::TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => { error!("HTTP server bind failed: {}", e); return; }
    };
    info!("HTTP API listening on {}", addr);

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                if let Err(e) = handle_http_request(&mut stream, &db, &query_engine, &db_manager) {
                    error!("HTTP request error: {}", e);
                }
            }
            Err(e) => { error!("HTTP accept error: {}", e); }
        }
    }
}

fn handle_http_request(stream: &mut std::net::TcpStream, db: &Arc<StorageEngine>, query_engine: &QueryEngine, db_manager: &Arc<MultiDbManager>) -> Result<()> {
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf)?;
    if n == 0 { return Ok(()); }

    let request_str = String::from_utf8_lossy(&buf[..n]);
    let (method, path, body) = parse_http_request(&request_str);

    let response = match (method.as_str(), path.as_str()) {
        ("GET", "/api/v1/ping") => http_response(200, "pong"),
        ("GET", "/api/v1/databases") => {
            let dbs = db_manager.list_databases();
            http_response(200, &format!(r#"{{"databases":{}}}"#, serde_json::to_string(&dbs).unwrap_or_default()))
        }
        ("POST", p) if p.starts_with("/api/v1/databases") => {
            match db_manager.create_database(&body.trim_matches('"')) {
                Ok(_) => http_response(201, r#"{"status":"created"}"#),
                Err(e) => http_response(500, &format!(r#"{{"error":"{}"}}"#, e)),
            }
        }
        ("DELETE", p) if p.starts_with("/api/v1/databases/") => {
            let name = p.strip_prefix("/api/v1/databases/").unwrap_or("");
            match db_manager.drop_database(name) {
                Ok(_) => http_response(200, r#"{"status":"dropped"}"#),
                Err(e) => http_response(500, &format!(r#"{{"error":"{}"}}"#, e)),
            }
        }
        ("POST", p) if p.starts_with("/api/v1/write") => {
            match handle_write(&body, db) {
                Ok(()) => http_response(204, ""),
                Err(e) => http_response(500, &format!(r#"{{"error":"{}"}}"#, e)),
            }
        }
        ("POST", p) if p.starts_with("/api/v1/query") => {
            match handle_query(&body, db, query_engine) {
                Ok(json) => http_response(200, &json),
                Err(e) => http_response(500, &format!(r#"{{"error":"{}"}}"#, e)),
            }
        }
        ("GET", p) if p.starts_with("/api/v1/chart") => {
            match handle_chart(&body, db, query_engine) {
                Ok(svg) => http_response_svg(200, &svg),
                Err(e) => http_response(500, &format!(r#"{{"error":"{}"}}"#, e)),
            }
        }
        ("POST", p) if p.starts_with("/api/v1/chart") => {
            match handle_chart(&body, db, query_engine) {
                Ok(svg) => http_response_svg(200, &svg),
                Err(e) => http_response(500, &format!(r#"{{"error":"{}"}}"#, e)),
            }
        }
        ("GET", p) if p.starts_with("/api/v1/timeseries") => {
            match handle_timeseries(&body) {
                Ok(svg) => http_response_svg(200, &svg),
                Err(e) => http_response(500, &format!(r#"{{"error":"{}"}}"#, e)),
            }
        }
        ("POST", p) if p.starts_with("/api/v1/timeseries") => {
            match handle_timeseries(&body) {
                Ok(svg) => http_response_svg(200, &svg),
                Err(e) => http_response(500, &format!(r#"{{"error":"{}"}}"#, e)),
            }
        }
        ("GET", "/api/v1/dashboard/business") => {
            match handle_business_dashboard(&body, db, query_engine) {
                Ok(html) => http_response_html(200, &html),
                Err(e) => http_response(500, &format!(r#"{{"error":"{}"}}"#, e)),
            }
        }
        ("GET", "/api/v1/dashboard/performance") => {
            match handle_performance_dashboard() {
                Ok(html) => http_response_html(200, &html),
                Err(e) => http_response(500, &format!(r#"{{"error":"{}"}}"#, e)),
            }
        }
        _ => http_response(404, r#"{"error":"not found"}"#),
    };

    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn handle_write(body: &str, db: &StorageEngine) -> Result<()> {
    let write_req: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| TsdbError::Serialization(format!("invalid JSON: {}", e)))?;

    let measurement = write_req["measurement"].as_str().unwrap_or("unknown");
    let timestamp = write_req["timestamp"].as_i64()
        .unwrap_or_else(|| chrono::Utc::now().timestamp_micros());

    let mut dp = DataPoint::new(measurement, timestamp);

    if let Some(tags) = write_req["tags"].as_object() {
        for (k, v) in tags {
            if let Some(s) = v.as_str() { dp.tags.insert(k.clone(), s.to_string()); }
        }
    }

    if let Some(fields) = write_req["fields"].as_object() {
        for (k, v) in fields {
            let fv = if let Some(f) = v.as_f64() { FieldValue::Float(f) }
                else if let Some(i) = v.as_i64() { FieldValue::Integer(i) }
                else if let Some(s) = v.as_str() { FieldValue::String(s.to_string()) }
                else if let Some(b) = v.as_bool() { FieldValue::Boolean(b) }
                else { continue };
            dp.fields.insert(k.clone(), fv);
        }
    }

    db.write(&dp)?;
    Ok(())
}

fn handle_query(body: &str, db: &StorageEngine, query_engine: &QueryEngine) -> Result<String> {
    let query_req: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| TsdbError::Serialization(format!("invalid JSON: {}", e)))?;

    let sql = query_req["sql"].as_str().unwrap_or("");
    let result = query_engine.execute(sql, db)
        .map_err(|e| TsdbError::Query(e.to_string()))?;

    let columns = &result.columns;
    let rows: Vec<Vec<serde_json::Value>> = result.rows.iter().map(|row| {
        row.iter().map(|fv| match fv {
            FieldValue::Float(f) => serde_json::json!(f),
            FieldValue::Integer(i) => serde_json::json!(i),
            FieldValue::String(s) => serde_json::json!(s),
            FieldValue::Boolean(b) => serde_json::json!(b),
        }).collect()
    }).collect();

    Ok(serde_json::json!({ "columns": columns, "rows": rows }).to_string())
}

fn handle_chart(body: &str, db: &StorageEngine, query_engine: &QueryEngine) -> Result<String> {
    let chart_req: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| TsdbError::Serialization(format!("invalid JSON: {}", e)))?;

    let sql = chart_req["sql"].as_str().unwrap_or("");
    let chart_type = chart_req["chart_type"].as_str().unwrap_or("line");
    let title = chart_req["title"].as_str().unwrap_or("");

    let result = query_engine.execute(sql, db)
        .map_err(|e| TsdbError::Query(e.to_string()))?;

    let mut chart = tsdb_chart::TimeSeriesChart::new(tsdb_chart::ChartConfig {
        title: title.to_string(),
        chart_type: match chart_type {
            "area" => tsdb_chart::ChartType::Area,
            "bar" => tsdb_chart::ChartType::Bar,
            _ => tsdb_chart::ChartType::Line,
        },
        ..Default::default()
    });

    if !result.rows.is_empty() {
        if let Some(ti) = result.columns.iter().position(|c| c == "time") {
            let mut series_map: std::collections::HashMap<String, tsdb_chart::TimeSeries> =
                std::collections::HashMap::new();
            for row in &result.rows {
                let ts = row[ti].as_i64().unwrap_or(0);
                for (ci, col) in result.columns.iter().enumerate() {
                    if ci == ti || col == "measurement" { continue; }
                    if let Some(v) = row[ci].as_f64() {
                        series_map.entry(col.clone())
                            .or_insert_with(|| tsdb_chart::TimeSeries::new(col))
                            .add_point(ts, v);
                    }
                }
            }
            for (_, series) in series_map { chart.add_series(series); }
        }
    }

    Ok(tsdb_chart::SvgRenderer::render(&chart))
}

fn parse_http_request(request: &str) -> (String, String, String) {
    let mut lines = request.split("\r\n");
    let first_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    let method = parts.first().unwrap_or(&"GET").to_string();
    let path = parts.get(1).unwrap_or(&"/").to_string();
    let body = if let Some(pos) = request.find("\r\n\r\n") {
        request[pos + 4..].trim_end_matches('\0').to_string()
    } else { String::new() };
    (method, path, body)
}

fn http_response(status: u16, body: &str) -> String {
    let status_text = match status { 200 => "OK", 201 => "Created", 204 => "No Content", 404 => "Not Found", 500 => "Internal Server Error", _ => "Unknown" };
    format!("HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}", status, status_text, body.len(), body)
}

fn http_response_svg(status: u16, body: &str) -> String {
    let status_text = if status == 200 { "OK" } else { "Unknown" };
    format!("HTTP/1.1 {} {}\r\nContent-Type: image/svg+xml\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}", status, status_text, body.len(), body)
}

fn http_response_html(status: u16, body: &str) -> String {
    let status_text = if status == 200 { "OK" } else { "Not Found" };
    format!("HTTP/1.1 {} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}", status, status_text, body.len(), body)
}

fn handle_business_dashboard(sql_str: &str, db: &Arc<StorageEngine>, query_engine: &QueryEngine) -> Result<String> {
    let result = query_engine.execute(sql_str, db).map_err(|e| TsdbError::Query(e.to_string()))?;
    let dash = tsdb_dashboard::BusinessDashboard::from_query_result(&result.columns, &result.rows);
    Ok(tsdb_dashboard::DashboardRenderer::render_business_html(&dash))
}

fn handle_timeseries(body: &str) -> Result<String> {
    let req: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| TsdbError::Serialization(format!("invalid JSON: {}", e)))?;

    let business = req["business"].as_str().unwrap_or("default");
    let measurement = req["measurement"].as_str().unwrap_or("cpu");
    let dimension = req["dimension"].as_str().unwrap_or("hour");
    let chart_type = req["chart_type"].as_str().unwrap_or("line");
    let title = req["title"].as_str().unwrap_or("Timeseries Trend");
    let start_ts = req["start_ts"].as_i64().unwrap_or(0);
    let end_ts = req["end_ts"].as_i64().unwrap_or(i64::MAX);

    let data_dir = std::env::var("TSDB_DATA_DIR")
        .unwrap_or_else(|_| "./data".to_string());
    let agg_dir = std::path::Path::new(&data_dir).join("aggregation");

    let store = tsdb_aggregate::store::AggregationStore::open(&agg_dir, business)
        .map_err(|e| TsdbError::Storage(e))?;

    let dim = tsdb_aggregate::aggregator::TimeDimension::from_name(dimension);
    let ct = match chart_type {
        "area" => tsdb_chart::ChartType::Area,
        "bar" => tsdb_chart::ChartType::Bar,
        _ => tsdb_chart::ChartType::Line,
    };

    tsdb_aggregate::TimeseriesGenerator::generate_trend(
        &store, business, measurement, dim, start_ts, end_ts, ct, title,
    ).map_err(|e| TsdbError::Storage(e))
}

fn handle_performance_dashboard() -> Result<String> {
    let mut dash = tsdb_dashboard::PerformanceDashboard::new();
    for gauge in tsdb_dashboard::PerformanceDashboard::default_gauges(50000.0, 120000.0, 5.2, 12.5) {
        dash.add_gauge(gauge);
    }
    dash.record(tsdb_dashboard::performance::TimestampRecord {
        timestamp: chrono::Utc::now().timestamp_micros(),
        writes: 50000, reads: 120000,
        bytes_written: 1024 * 1024 * 512, bytes_read: 1024 * 1024 * 2048,
    });
    Ok(tsdb_dashboard::DashboardRenderer::render_performance_html(&dash))
}
