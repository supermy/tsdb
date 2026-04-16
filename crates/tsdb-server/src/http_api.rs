//! # TSDB HTTP API — RESTful 接口服务
//!
//! 基于 warp 框架的异步 HTTP 服务，提供 RESTful 风格的数据读写接口。
//! 所有路由均使用 async/await，支持高并发请求处理。

use serde_json::json;
use std::sync::Arc;
use tracing::info;
use tsdb_core::storage::multi_db::MultiDbManager;
use tsdb_core::storage::StorageEngine;
use tsdb_query::QueryEngine;
use tsdb_types::model::{DataPoint, FieldValue};
use warp::{reply, Filter, Rejection, Reply};

/// 启动异步 HTTP 服务器（warp）
pub async fn start_http_server_async(
    addr: &str,
    _db: &StorageEngine,
    query_engine: &QueryEngine,
    db_manager: &Arc<MultiDbManager>,
) {
    let qe = query_engine.clone();
    let db_mgr = Arc::clone(db_manager);

    let write_route = warp::path!("api" / "v1" / "write")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_state(db_mgr.clone()))
        .and_then(handle_write);

    let query_route = warp::path!("api" / "v1" / "query")
        .and(warp::get())
        .and(warp::query::<QueryParams>())
        .and(with_state(db_mgr.clone()))
        .and(with_query_engine(qe))
        .and_then(handle_query);

    let timeseries_route = warp::path!("api" / "v1" / "timeseries")
        .and(warp::method())
        .and(with_state(db_mgr.clone()))
        .and_then(handle_timeseries);

    let create_db_route = warp::path!("api" / "v1" / "databases" / String)
        .and(warp::post())
        .and(with_state(db_mgr.clone()))
        .and_then(handle_create_database);

    let drop_db_route = warp::path!("api" / "v1" / "databases" / String)
        .and(warp::delete())
        .and(with_state(db_mgr.clone()))
        .and_then(handle_drop_database);

    let list_db_route = warp::path!("api" / "v1" / "databases")
        .and(warp::get())
        .and(with_state(db_mgr.clone()))
        .and_then(handle_list_databases);

    let health_route =
        warp::path("health").map(|| reply::json(&json!({"status": "ok", "service": "tsdb-http"})));

    let routes = write_route
        .or(query_route)
        .or(timeseries_route)
        .or(create_db_route)
        .or(drop_db_route)
        .or(list_db_route)
        .or(health_route)
        .with(
            warp::cors()
                .allow_any_origin()
                .allow_methods(vec!["GET", "POST"])
                .allow_headers(vec!["content-type"]),
        );

    let socket_addr: std::net::SocketAddr = addr
        .parse()
        .map_err(|e| {
            tracing::error!("invalid HTTP address '{}': {}", addr, e);
            e
        })
        .unwrap_or_else(|e| panic!("invalid HTTP address '{}': {}", addr, e));

    info!("HTTP server starting on {}", addr);
    warp::serve(routes).run(socket_addr).await;
}

fn with_state(
    state: Arc<MultiDbManager>,
) -> impl Filter<Extract = (Arc<MultiDbManager>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || state.clone())
}

fn with_query_engine(
    qe: QueryEngine,
) -> impl Filter<Extract = (QueryEngine,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || qe.clone())
}

#[derive(serde::Deserialize)]
struct QueryParams {
    sql: String,
}

async fn handle_write(
    body: serde_json::Value,
    db_mgr: Arc<MultiDbManager>,
) -> Result<impl Reply, Rejection> {
    match body.get("database").and_then(|v| v.as_str()) {
        Some(db_name) => {
            let db = db_mgr
                .get_database(db_name)
                .map_err(|e| warp::reject::custom(HttpError(e.to_string())))?;
            handle_write_to_db(body, &db).await
        }
        None => {
            let db = db_mgr
                .get_database("default")
                .map_err(|e| warp::reject::custom(HttpError(e.to_string())))?;
            handle_write_to_db(body, &db).await
        }
    }
}

async fn handle_write_to_db(
    body: serde_json::Value,
    db: &StorageEngine,
) -> Result<impl Reply, Rejection> {
    let measurement = body
        .get("measurement")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let timestamp = body.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
    let tags_obj = body.get("tags").cloned().unwrap_or(json!({}));
    let fields_obj = body.get("fields").cloned().unwrap_or(json!({}));

    let mut dp = DataPoint::new(&measurement, timestamp);

    if let Some(tags_map) = tags_obj.as_object() {
        for (k, v) in tags_map {
            if let Some(s) = v.as_str() {
                dp.tags.insert(k.clone(), s.to_string());
            }
        }
    }

    if let Some(fields_map) = fields_obj.as_object() {
        for (k, v) in fields_map {
            dp.fields.insert(
                k.clone(),
                v.as_f64()
                    .map(FieldValue::Float)
                    .or_else(|| v.as_i64().map(FieldValue::Integer))
                    .or_else(|| v.as_str().map(|s| FieldValue::String(s.to_string())))
                    .unwrap_or(FieldValue::Float(f64::NAN)),
            );
        }
    }

    match db.write(&dp) {
        Ok(_) => Ok(reply::with_header(
            reply::json(&json!({"status": "ok", "message": "data written"})),
            "content-type",
            "application/json",
        )),
        Err(e) => Err(warp::reject::custom(HttpError(e.to_string()))),
    }
}

async fn handle_query(
    params: QueryParams,
    db_mgr: Arc<MultiDbManager>,
    qe: QueryEngine,
) -> Result<impl Reply, Rejection> {
    let db = db_mgr
        .get_database("default")
        .map_err(|e| warp::reject::custom(HttpError(e.to_string())))?;

    let result = qe
        .execute(&params.sql, &db)
        .map_err(|e| warp::reject::custom(HttpError(e.to_string())))?;

    let columns = result.columns;
    let rows: Vec<serde_json::Value> = result
        .rows
        .into_iter()
        .map(|row| {
            let vals: Vec<serde_json::Value> = row.into_iter().map(|v| v.into()).collect();
            json!(vals)
        })
        .collect();

    Ok(reply::with_header(
        reply::json(&json!({
            "columns": columns,
            "rows": rows,
            "count": rows.len(),
        })),
        "content-type",
        "application/json",
    ))
}

async fn handle_timeseries(
    _method: warp::http::Method,
    _db_mgr: Arc<MultiDbManager>,
) -> std::result::Result<warp::reply::WithHeader<warp::reply::Json>, warp::Rejection> {
    use tsdb_aggregate::{aggregator::TimeDimension, AggregationStoreManager, TimeseriesGenerator};

    let store_mgr = AggregationStoreManager::new("/tmp/tsdb_aggregations".into());

    let store: Arc<tsdb_aggregate::AggregationStore> = store_mgr
        .get_store("default")
        .map_err(|e| warp::reject::custom(HttpError(e)))?;

    let svg = TimeseriesGenerator::generate_trend(
        &store,
        "default",
        "cpu",
        TimeDimension::Hour,
        0,
        i64::MAX,
        tsdb_chart::ChartType::Area,
        "TSDB Trend",
    )
    .map_err(|e: String| warp::reject::custom(HttpError(e)))?;

    Ok(reply::with_header(
        reply::json(&json!({"svg": svg})),
        "content-type",
        "application/json",
    ))
}

async fn handle_create_database(
    name: String,
    db_mgr: Arc<MultiDbManager>,
) -> Result<impl Reply, Rejection> {
    match db_mgr.create_database(&name) {
        Ok(_) => Ok(reply::with_header(
            reply::json(
                &json!({"status": "ok", "message": format!("database '{}' created", name)}),
            ),
            "content-type",
            "application/json",
        )),
        Err(e) => Err(warp::reject::custom(HttpError(e.to_string()))),
    }
}

async fn handle_drop_database(
    name: String,
    db_mgr: Arc<MultiDbManager>,
) -> Result<impl Reply, Rejection> {
    match db_mgr.drop_database(&name) {
        Ok(_) => Ok(reply::with_header(
            reply::json(
                &json!({"status": "ok", "message": format!("database '{}' dropped", name)}),
            ),
            "content-type",
            "application/json",
        )),
        Err(e) => Err(warp::reject::custom(HttpError(e.to_string()))),
    }
}

async fn handle_list_databases(db_mgr: Arc<MultiDbManager>) -> Result<impl Reply, Rejection> {
    let databases = db_mgr.list_databases();
    Ok(reply::with_header(
        reply::json(&json!({"databases": databases})),
        "content-type",
        "application/json",
    ))
}

#[derive(Debug)]
struct HttpError(String);
impl Reply for HttpError {
    fn into_response(self) -> warp::reply::Response {
        reply::with_status(
            reply::json(&json!({"error": self.0})),
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        )
        .into_response()
    }
}
impl warp::reject::Reject for HttpError {}
