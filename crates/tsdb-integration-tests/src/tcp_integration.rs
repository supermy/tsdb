//! TCP 客户端-服务端集成测试
//!
//! 测试覆盖：
//! 1. TCP 连接 → Ping/Pong 握手

#![allow(dead_code, unused_imports)]

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use tsdb_config::{AggregateConfig, LogConfig, ServerConfig, StorageConfig, TsdbConfig};
use tsdb_server::{protocol, TsdbServer};

#[allow(dead_code)]
fn make_test_config(port: u16) -> TsdbConfig {
    TsdbConfig {
        server: ServerConfig {
            host: "127.0.0.1".to_string(),
            port,
            workers: 2,
        },
        storage: StorageConfig {
            data_dir: std::env::temp_dir().join(format!("tsdb_tcp_test_{}", port)),
            hot_days: 7,
            retention_days: 30,
            block_duration_secs: 30,
            write_buffer_size: 64 * 1024 * 1024,
            max_open_files: 1000,
        },
        aggregate: AggregateConfig {
            enabled: false,
            worker_count: 0,
            time_dimensions: vec![],
            nng_url: String::new(),
        },
        log: LogConfig {
            level: "warn".to_string(),
            file: None,
        },
    }
}

#[allow(dead_code)]
async fn send_request(stream: &mut TcpStream, req: &protocol::Request) -> protocol::Response {
    let data = protocol::encode_request(req);
    let len = data.len() as u32;

    stream.write_all(&len.to_be_bytes()).await.unwrap();
    stream.write_all(&data).await.unwrap();
    stream.flush().await.unwrap();

    let mut len_buf = [0u8; 4];
    timeout(Duration::from_secs(5), stream.read_exact(&mut len_buf))
        .await
        .expect("read response length timeout")
        .unwrap();

    let resp_len = u32::from_be_bytes(len_buf) as usize;
    let mut resp_data = vec![0u8; resp_len];
    timeout(Duration::from_secs(5), stream.read_exact(&mut resp_data))
        .await
        .expect("read response body timeout")
        .unwrap();

    protocol::decode_response(&resp_data).expect("decode response failed")
}

#[allow(dead_code)]
async fn with_server<F, Fut>(port: u16, f: F)
where
    F: FnOnce(u16) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let config = make_test_config(port);
    let server = Arc::new(TsdbServer::new(config));

    let server_handle = {
        let s = Arc::clone(&server);
        tokio::spawn(async move {
            let _ = s.start().await;
        })
    };

    tokio::time::sleep(Duration::from_millis(500)).await;
    f(port).await;

    server_handle.abort();
}

#[tokio::test]
async fn test_ping_pong() {
    with_server(17890, |port| async move {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
            .await
            .unwrap();

        let resp = send_request(&mut stream, &protocol::Request::Ping).await;
        assert!(matches!(resp, protocol::Response::Pong));
    })
    .await;
}
