//! # TSDB 服务器核心 — TCP 二进制协议服务
//!
//! TsdbServer 是 TSDB 的主入口点，提供基于 TCP 的自定义二进制协议服务，
//! 支持多业务数据库隔离。

use crate::protocol::{decode_request, encode_response, Request, Response};
use std::io::{Read, Write};
use std::sync::Arc;
use tracing::{error, info};
use tsdb_config::TsdbConfig;
use tsdb_core::error::{Result, TsdbError};
use tsdb_core::storage::cf_manager::CfConfig;
use tsdb_core::storage::multi_db::MultiDbManager;
use tsdb_query::QueryEngine;
use tsdb_types::model::DataPoint;

/// TSDB 服务器实例 — 管理多业务数据库连接、处理客户端请求
pub struct TsdbServer {
    config: TsdbConfig,
    db_manager: Arc<MultiDbManager>,
    query_engine: QueryEngine,
}

impl TsdbServer {
    pub fn new(config: TsdbConfig) -> Self {
        let cf_config = CfConfig {
            hot_days: config.storage.hot_days,
            retention_days: config.storage.retention_days,
        };
        let db_manager = Arc::new(MultiDbManager::new(
            config.storage.data_dir.clone(),
            cf_config,
        ));
        Self {
            config,
            db_manager,
            query_engine: QueryEngine::new(),
        }
    }

    pub fn start(&mut self) -> Result<()> {
        let addr = format!("{}:{}", self.config.server.host, self.config.server.port);
        info!("TSDB server starting on {}", addr);

        self.db_manager.ensure_default()?;

        let http_port = self.config.server.port + 1;
        info!(
            "HTTP API available at http://{}:{}/api/v1/",
            self.config.server.host, http_port
        );

        let listener = std::net::TcpListener::bind(&addr)
            .map_err(|e| TsdbError::Network(format!("failed to bind {}: {}", addr, e)))?;

        info!("TSDB server listening on {}", addr);

        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    if let Err(e) = self.handle_connection(&mut stream) {
                        error!("connection error: {}", e);
                    }
                }
                Err(e) => {
                    error!("accept error: {}", e);
                }
            }
        }
        Ok(())
    }

    pub fn start_with_http(&mut self) -> Result<()> {
        let http_port = self.config.server.port + 1;
        let http_addr = format!("{}:{}", self.config.server.host, http_port);

        self.db_manager.ensure_default()?;

        let tcp_addr = format!("{}:{}", self.config.server.host, self.config.server.port);
        let listener = std::net::TcpListener::bind(&tcp_addr)
            .map_err(|e| TsdbError::Network(format!("bind failed: {}", e)))?;

        info!("TSDB server listening on {}", tcp_addr);
        info!("HTTP API at http://{}/api/v1/", http_addr);

        let db = self.db_manager.get_database("default")?;
        let query_engine = QueryEngine::new();
        let db_mgr = Arc::clone(&self.db_manager);
        std::thread::spawn(move || {
            crate::http_api::start_http_server(&http_addr, db, query_engine, db_mgr);
        });

        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    if let Err(e) = self.handle_connection(&mut stream) {
                        error!("connection error: {}", e);
                    }
                }
                Err(e) => {
                    error!("accept error: {}", e);
                }
            }
        }
        Ok(())
    }

    pub fn start_with_nng(&mut self, rep_port: u16, pull_port: u16, pub_port: u16) -> Result<()> {
        self.db_manager.ensure_default()?;

        let db_mgr = Arc::clone(&self.db_manager);
        let query_engine = QueryEngine::new();

        let rep_url = format!("tcp://*:{}", rep_port);
        let pull_url = format!("tcp://*:{}", pull_port);
        let pub_url = format!("tcp://*:{}", pub_port);

        let nng_server = crate::nng_transport::NngServer::new(
            &rep_url,
            &pub_url,
            &pull_url,
            Arc::clone(&db_mgr),
            query_engine,
        );

        std::thread::spawn(move || {
            if let Err(e) = nng_server.start_rep() {
                error!("NNG REP error: {}", e);
            }
        });

        let nng_pull_server = crate::nng_transport::NngServer::new(
            &rep_url,
            &pub_url,
            &pull_url,
            db_mgr,
            QueryEngine::new(),
        );
        std::thread::spawn(move || {
            if let Err(e) = nng_pull_server.start_pull() {
                error!("NNG PULL error: {}", e);
            }
        });

        info!("NNG REP listening on {}", rep_url);
        info!("NNG PULL listening on {}", pull_url);
        info!("NNG PUB listening on {}", pub_url);

        Ok(())
    }

    pub fn start_all(&mut self) -> Result<()> {
        let http_port = self.config.server.port + 1;
        let nng_rep_port = self.config.server.port + 2;
        let nng_pull_port = self.config.server.port + 3;
        let nng_pub_port = self.config.server.port + 4;

        self.start_with_nng(nng_rep_port, nng_pull_port, nng_pub_port)?;

        let http_addr = format!("{}:{}", self.config.server.host, http_port);
        let tcp_addr = format!("{}:{}", self.config.server.host, self.config.server.port);
        let listener = std::net::TcpListener::bind(&tcp_addr)
            .map_err(|e| TsdbError::Network(format!("bind failed: {}", e)))?;

        info!("TSDB server listening on {}", tcp_addr);
        info!("HTTP API at http://{}/api/v1/", http_addr);

        let db = self.db_manager.get_database("default")?;
        let query_engine = QueryEngine::new();
        let db_mgr = Arc::clone(&self.db_manager);
        std::thread::spawn(move || {
            crate::http_api::start_http_server(&http_addr, db, query_engine, db_mgr);
        });

        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    if let Err(e) = self.handle_connection(&mut stream) {
                        error!("connection error: {}", e);
                    }
                }
                Err(e) => {
                    error!("accept error: {}", e);
                }
            }
        }
        Ok(())
    }

    fn handle_connection(&self, stream: &mut std::net::TcpStream) -> Result<()> {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut data = vec![0u8; len];
        stream.read_exact(&mut data)?;

        let request = decode_request(&data)
            .ok_or_else(|| TsdbError::Protocol("failed to decode request".into()))?;

        let response = self.process_request(request);

        let resp_data = encode_response(&response);
        let resp_len = resp_data.len() as u32;
        stream.write_all(&resp_len.to_be_bytes())?;
        stream.write_all(&resp_data)?;
        stream.flush()?;
        Ok(())
    }

    fn process_request(&self, request: Request) -> Response {
        match request {
            Request::Ping => Response::Pong,

            Request::Write {
                database,
                measurement,
                tags,
                fields,
                timestamp,
            } => {
                let db_name = if database.is_empty() {
                    "default"
                } else {
                    &database
                };
                match self.db_manager.get_database(db_name) {
                    Ok(db) => {
                        let mut dp = DataPoint::new(measurement, timestamp);
                        for (k, v) in tags {
                            dp.tags.insert(k, v);
                        }
                        for (k, v) in fields {
                            dp.fields.insert(k, v.into());
                        }
                        match db.write(&dp) {
                            Ok(()) => Response::Ok,
                            Err(e) => Response::Error(e.to_string()),
                        }
                    }
                    Err(e) => Response::Error(e.to_string()),
                }
            }

            Request::Query { database, sql } => {
                let db_name = if database.is_empty() {
                    "default"
                } else {
                    &database
                };
                match self.db_manager.get_database(db_name) {
                    Ok(db) => match self.query_engine.execute(&sql, &db) {
                        Ok(result) => {
                            let columns = result.columns;
                            let rows = result
                                .rows
                                .into_iter()
                                .map(|row| row.into_iter().map(|v| v.into()).collect())
                                .collect();
                            Response::QueryResult { columns, rows }
                        }
                        Err(e) => Response::Error(e.to_string()),
                    },
                    Err(e) => Response::Error(e.to_string()),
                }
            }

            Request::CreateDatabase { name } => match self.db_manager.create_database(&name) {
                Ok(_) => Response::Ok,
                Err(e) => Response::Error(e.to_string()),
            },

            Request::ListDatabases => Response::Databases(self.db_manager.list_databases()),

            Request::DropDatabase { name } => match self.db_manager.drop_database(&name) {
                Ok(_) => Response::Ok,
                Err(e) => Response::Error(e.to_string()),
            },
        }
    }
}
