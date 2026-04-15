use crate::protocol::{Request, Response, decode_request, encode_response};
use tsdb_core::storage::StorageEngine;
use tsdb_core::error::TsdbError;
use tsdb_query::QueryEngine;
use tsdb_types::model::{DataPoint, FieldValue};
use std::sync::Arc;
use tracing::{info, error};

pub struct NngServer {
    rep_url: String,
    pub_url: String,
    pull_url: String,
    db: Arc<StorageEngine>,
    query_engine: QueryEngine,
}

impl NngServer {
    pub fn new(
        rep_url: &str,
        pub_url: &str,
        pull_url: &str,
        db: Arc<StorageEngine>,
        query_engine: QueryEngine,
    ) -> Self {
        Self {
            rep_url: rep_url.to_string(),
            pub_url: pub_url.to_string(),
            pull_url: pull_url.to_string(),
            db,
            query_engine,
        }
    }

    pub fn start_rep(&self) -> Result<(), TsdbError> {
        let socket = nng::Socket::new(nng::Protocol::Rep0)
            .map_err(|e| TsdbError::Nng(format!("open rep failed: {:?}", e)))?;

        socket.listen(&self.rep_url)
            .map_err(|e| TsdbError::Nng(format!("listen rep failed: {:?}", e)))?;

        info!("NNG REP server listening on {}", self.rep_url);

        loop {
            let msg = socket.recv()
                .map_err(|e| TsdbError::Nng(format!("recv failed: {:?}", e)))?;

            let request = decode_request(msg.as_slice());
            let response = if let Some(req) = request {
                self.process_request(req)
            } else {
                Response::Error("invalid request".into())
            };

            let resp_data = encode_response(&response);
            let mut reply = nng::Message::new();
            reply.push_back(&resp_data);

            socket.send(reply)
                .map_err(|e| TsdbError::Nng(format!("send failed: {:?}", e)))?;
        }
    }

    pub fn start_pull(&self) -> Result<(), TsdbError> {
        let socket = nng::Socket::new(nng::Protocol::Pull0)
            .map_err(|e| TsdbError::Nng(format!("open pull failed: {:?}", e)))?;

        socket.listen(&self.pull_url)
            .map_err(|e| TsdbError::Nng(format!("listen pull failed: {:?}", e)))?;

        info!("NNG PULL server listening on {}", self.pull_url);

        loop {
            let msg = socket.recv()
                .map_err(|e| TsdbError::Nng(format!("pull recv failed: {:?}", e)))?;

            let request = decode_request(msg.as_slice());
            if let Some(Request::Write { measurement, tags, fields, timestamp }) = request {
                let mut dp = DataPoint::new(measurement, timestamp);
                for (k, v) in tags {
                    dp.tags.insert(k, v);
                }
                for (k, v) in fields {
                    dp.fields.insert(k, v.into());
                }
                if let Err(e) = self.db.write(&dp) {
                    error!("NNG pull write error: {}", e);
                }
            }
        }
    }

    fn process_request(&self, request: Request) -> Response {
        match request {
            Request::Ping => Response::Pong,

            Request::Write { measurement, tags, fields, timestamp } => {
                let mut dp = DataPoint::new(measurement, timestamp);
                for (k, v) in tags {
                    dp.tags.insert(k, v);
                }
                for (k, v) in fields {
                    dp.fields.insert(k, v.into());
                }
                match self.db.write(&dp) {
                    Ok(()) => Response::Ok,
                    Err(e) => Response::Error(e.to_string()),
                }
            }

            Request::Query { sql } => {
                match self.query_engine.execute(&sql, &self.db) {
                    Ok(result) => {
                        let columns = result.columns;
                        let rows = result.rows.into_iter()
                            .map(|row| row.into_iter().map(|v| v.into()).collect())
                            .collect();
                        Response::QueryResult { columns, rows }
                    }
                    Err(e) => Response::Error(e.to_string()),
                }
            }

            Request::ListDatabases => {
                Response::Databases(vec!["default".to_string()])
            }

            _ => Response::Error("not supported via NNG".into()),
        }
    }
}

pub struct NngPublisher {
    socket: nng::Socket,
}

impl NngPublisher {
    pub fn new(url: &str) -> Result<Self, TsdbError> {
        let socket = nng::Socket::new(nng::Protocol::Pub0)
            .map_err(|e| TsdbError::Nng(format!("open pub failed: {:?}", e)))?;

        socket.listen(url)
            .map_err(|e| TsdbError::Nng(format!("listen pub failed: {:?}", e)))?;

        info!("NNG PUB server listening on {}", url);

        Ok(Self { socket })
    }

    pub fn publish(&self, topic: &str, data: &[u8]) -> Result<(), TsdbError> {
        let mut msg = nng::Message::new();
        msg.push_back(topic.as_bytes());
        msg.push_back(&[0u8]);
        msg.push_back(data);

        self.socket.send(msg)
            .map_err(|e| TsdbError::Nng(format!("publish failed: {:?}", e)))?;

        Ok(())
    }
}
