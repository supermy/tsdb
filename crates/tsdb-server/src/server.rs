use crate::protocol::{Request, Response, FieldValueProto, decode_request, encode_response};
use tsdb_core::storage::StorageEngine;
use tsdb_core::storage::cf_manager::CfConfig;
use tsdb_config::TsdbConfig;
use tsdb_query::QueryEngine;
use tsdb_types::model::{DataPoint, FieldValue};
use std::collections::HashMap;
use std::sync::Arc;
use std::io::{Read, Write};
use tracing::{info, error, warn};

pub struct TsdbServer {
    config: TsdbConfig,
    databases: HashMap<String, Arc<StorageEngine>>,
    query_engine: QueryEngine,
}

impl TsdbServer {
    pub fn new(config: TsdbConfig) -> Self {
        Self {
            config,
            databases: HashMap::new(),
            query_engine: QueryEngine::new(),
        }
    }

    pub fn start(&mut self) -> anyhow::Result<()> {
        let addr = format!("{}:{}", self.config.server.host, self.config.server.port);
        info!("TSDB server starting on {}", addr);

        self.ensure_default_database()?;

        let http_port = self.config.server.port + 1;
        let http_addr = format!("{}:{}", self.config.server.host, http_port);
        info!("HTTP API available at http://{}/api/v1/", http_addr);

        let listener = std::net::TcpListener::bind(&addr)
            .map_err(|e| anyhow::anyhow!("failed to bind {}: {}", addr, e))?;

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

    pub fn start_with_http(&mut self) -> anyhow::Result<()> {
        let http_port = self.config.server.port + 1;
        let http_addr = format!("{}:{}", self.config.server.host, http_port);

        self.ensure_default_database()?;

        let tcp_addr = format!("{}:{}", self.config.server.host, self.config.server.port);
        let listener = std::net::TcpListener::bind(&tcp_addr)?;

        info!("TSDB server listening on {}", tcp_addr);
        info!("HTTP API at http://{}/api/v1/", http_addr);

        let db = self.databases.get("default").cloned();
        if let Some(db) = db {
            let query_engine = QueryEngine::new();
            std::thread::spawn(move || {
                crate::http_api::start_http_server(&http_addr, db, query_engine);
            });
        }

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

    fn handle_connection(&self, stream: &mut std::net::TcpStream) -> anyhow::Result<()> {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut data = vec![0u8; len];
        stream.read_exact(&mut data)?;

        let request = decode_request(&data)
            .ok_or_else(|| anyhow::anyhow!("failed to decode request"))?;

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

            Request::Write { measurement, tags, fields, timestamp } => {
                let db = self.databases.get("default");
                if let Some(db) = db {
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
                } else {
                    Response::Error("no default database".into())
                }
            }

            Request::Query { sql } => {
                let db = self.databases.get("default");
                if let Some(db) = db {
                    match self.query_engine.execute(&sql, db) {
                        Ok(result) => {
                            let columns = result.columns;
                            let rows = result.rows.into_iter()
                                .map(|row| row.into_iter().map(|v| v.into()).collect())
                                .collect();
                            Response::QueryResult { columns, rows }
                        }
                        Err(e) => Response::Error(e.to_string()),
                    }
                } else {
                    Response::Error("no default database".into())
                }
            }

            Request::CreateDatabase { name: _ } => {
                Response::Error("create database not yet implemented".into())
            }

            Request::ListDatabases => {
                Response::Databases(self.databases.keys().cloned().collect())
            }

            Request::DropDatabase { name: _ } => {
                Response::Error("drop database not yet implemented".into())
            }
        }
    }

    fn ensure_default_database(&mut self) -> anyhow::Result<()> {
        let data_dir = self.config.storage.data_dir.join("default");
        std::fs::create_dir_all(&data_dir)?;

        let cf_config = CfConfig {
            hot_days: self.config.storage.hot_days,
            retention_days: self.config.storage.retention_days,
        };

        let engine = StorageEngine::open(&data_dir, cf_config)?;
        self.databases.insert("default".to_string(), Arc::new(engine));

        info!("default database opened at {:?}", data_dir);
        Ok(())
    }
}
