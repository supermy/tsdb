//! # NNG 传输层 — REP/PULL/PUB 三协议模式

use crate::protocol::{Request, Response, decode_request, encode_response};
use tsdb_core::storage::StorageEngine;
use tsdb_core::storage::multi_db::MultiDbManager;
use tsdb_core::error::TsdbError;
use tsdb_query::QueryEngine;
use tsdb_types::model::{DataPoint, FieldValue};
use std::sync::Arc;
use tracing::{info, error};

/// NNG REP/PULL 双协议服务器
pub struct NngServer {
    rep_url: String,
    pub_url: String,
    pull_url: String,
    db_manager: Arc<MultiDbManager>,
    query_engine: QueryEngine,
}

impl NngServer {
    pub fn new(rep_url: &str, pub_url: &str, pull_url: &str, db_manager: Arc<MultiDbManager>, query_engine: QueryEngine) -> Self {
        Self { rep_url: rep_url.to_string(), pub_url: pub_url.to_string(), pull_url: pull_url.to_string(), db_manager, query_engine }
    }

    pub fn start_rep(&self) -> Result<(), TsdbError> {
        let socket = nng::Socket::new(nng::Protocol::Rep0).map_err(|e| TsdbError::Nng(format!("open rep failed: {:?}", e)))?;
        socket.listen(&self.rep_url).map_err(|e| TsdbError::Nng(format!("listen rep failed: {:?}", e)))?;
        info!("NNG REP server listening on {}", self.rep_url);

        loop {
            let msg = socket.recv().map_err(|e| TsdbError::Nng(format!("recv failed: {:?}", e)))?;
            let request = decode_request(msg.as_slice());
            let response = if let Some(req) = request { self.process_request(req) } else { Response::Error("invalid request".into()) };
            let resp_data = encode_response(&response);
            let mut reply = nng::Message::new();
            reply.push_back(&resp_data);
            socket.send(reply).map_err(|e| TsdbError::Nng(format!("send failed: {:?}", e)))?;
        }
    }

    pub fn start_pull(&self) -> Result<(), TsdbError> {
        let socket = nng::Socket::new(nng::Protocol::Pull0).map_err(|e| TsdbError::Nng(format!("open pull failed: {:?}", e)))?;
        socket.listen(&self.pull_url).map_err(|e| TsdbError::Nng(format!("listen pull failed: {:?}", e)))?;
        info!("NNG PULL server listening on {}", self.pull_url);

        loop {
            let msg = socket.recv().map_err(|e| TsdbError::Nng(format!("pull recv failed: {:?}", e)))?;
            let request = decode_request(msg.as_slice());
            if let Some(Request::Write { database, measurement, tags, fields, timestamp }) = request {
                let db_name = if database.is_empty() { "default" } else { &database };
                if let Ok(db) = self.db_manager.get_database(db_name) {
                    let mut dp = DataPoint::new(measurement, timestamp);
                    for (k, v) in tags { dp.tags.insert(k, v); }
                    for (k, v) in fields { dp.fields.insert(k, v.into()); }
                    if let Err(e) = db.write(&dp) { error!("NNG pull write error: {}", e); }
                }
            }
        }
    }

    fn process_request(&self, request: Request) -> Response {
        match request {
            Request::Ping => Response::Pong,
            Request::Write { database, measurement, tags, fields, timestamp } => {
                let db_name = if database.is_empty() { "default" } else { &database };
                match self.db_manager.get_database(db_name) {
                    Ok(db) => {
                        let mut dp = DataPoint::new(measurement, timestamp);
                        for (k, v) in tags { dp.tags.insert(k, v); }
                        for (k, v) in fields { dp.fields.insert(k, v.into()); }
                        match db.write(&dp) { Ok(()) => Response::Ok, Err(e) => Response::Error(e.to_string()), }
                    }
                    Err(e) => Response::Error(e.to_string()),
                }
            }
            Request::Query { database, sql } => {
                let db_name = if database.is_empty() { "default" } else { &database };
                match self.db_manager.get_database(db_name) {
                    Ok(db) => match self.query_engine.execute(&sql, &db) {
                        Ok(result) => Response::QueryResult { columns: result.columns, rows: result.rows.into_iter().map(|row| row.into_iter().map(|v| v.into()).collect()).collect() },
                        Err(e) => Response::Error(e.to_string()),
                    },
                    Err(e) => Response::Error(e.to_string()),
                }
            }
            Request::CreateDatabase { name } => match self.db_manager.create_database(&name) {
                Ok(_) => Response::Ok, Err(e) => Response::Error(e.to_string()),
            },
            Request::ListDatabases => Response::Databases(self.db_manager.list_databases()),
            Request::DropDatabase { name } => match self.db_manager.drop_database(&name) {
                Ok(_) => Response::Ok, Err(e) => Response::Error(e.to_string()),
            },
        }
    }
}

/// NNG 发布者
pub struct NngPublisher { socket: nng::Socket }

impl NngPublisher {
    pub fn new(url: &str) -> Result<Self, TsdbError> {
        let socket = nng::Socket::new(nng::Protocol::Pub0).map_err(|e| TsdbError::Nng(format!("open pub failed: {:?}", e)))?;
        socket.listen(url).map_err(|e| TsdbError::Nng(format!("listen pub failed: {:?}", e)))?;
        info!("NNG PUB server listening on {}", url);
        Ok(Self { socket })
    }

    pub fn publish(&self, topic: &str, data: &[u8]) -> Result<(), TsdbError> {
        let mut msg = nng::Message::new();
        msg.push_back(topic.as_bytes());
        msg.push_back(&[0u8]);
        msg.push_back(data);
        self.socket.send(msg).map_err(|e| TsdbError::Nng(format!("publish failed: {:?}", e)))
    }
}
