use clap::{Parser, Subcommand};
use tsdb_config::TsdbConfig;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "tsdb-cli", version = "0.1.0", about = "TSDB command line client")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    #[arg(long, default_value_t = 7878)]
    port: u16,
}

#[derive(Subcommand)]
enum Commands {
    Start {
        #[arg(long, default_value = "config.ini")]
        config: PathBuf,
    },
    Query {
        sql: String,
    },
    Write {
        #[arg(long)]
        measurement: String,
        #[arg(long)]
        tags: Option<String>,
        #[arg(long)]
        fields: String,
        #[arg(long, default_value_t = 0)]
        timestamp: i64,
    },
    Ping,
    List,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start { config } => {
            let config = TsdbConfig::load_or_default(&config);
            let mut server = tsdb_server::TsdbServer::new(config);
            server.start()?;
        }
        Commands::Query { sql } => {
            println!("Query: {}", sql);
            let addr = format!("{}:{}", cli.host, cli.port);
            send_request(&addr, tsdb_server::protocol::Request::Query { sql })?;
        }
        Commands::Write { measurement, tags, fields, timestamp } => {
            let tag_pairs = parse_kv_pairs(tags.as_deref());
            let field_pairs = parse_field_values(&fields);
            let ts = if timestamp == 0 {
                chrono::Utc::now().timestamp_micros()
            } else {
                timestamp
            };
            let addr = format!("{}:{}", cli.host, cli.port);
            send_request(&addr, tsdb_server::protocol::Request::Write {
                measurement,
                tags: tag_pairs,
                fields: field_pairs,
                timestamp: ts,
            })?;
        }
        Commands::Ping => {
            let addr = format!("{}:{}", cli.host, cli.port);
            send_request(&addr, tsdb_server::protocol::Request::Ping)?;
        }
        Commands::List => {
            let addr = format!("{}:{}", cli.host, cli.port);
            send_request(&addr, tsdb_server::protocol::Request::ListDatabases)?;
        }
    }

    Ok(())
}

fn parse_kv_pairs(input: Option<&str>) -> Vec<(String, String)> {
    input
        .map(|s| {
            s.split(',')
                .filter_map(|pair| {
                    let mut parts = pair.splitn(2, '=');
                    Some((parts.next()?.to_string(), parts.next()?.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_field_values(input: &str) -> Vec<(String, tsdb_server::protocol::FieldValueProto)> {
    input
        .split(',')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next()?.to_string();
            let val_str = parts.next()?;
            let value = if let Ok(f) = val_str.parse::<f64>() {
                tsdb_server::protocol::FieldValueProto::Float(f)
            } else if let Ok(i) = val_str.parse::<i64>() {
                tsdb_server::protocol::FieldValueProto::Integer(i)
            } else if val_str == "true" {
                tsdb_server::protocol::FieldValueProto::Boolean(true)
            } else if val_str == "false" {
                tsdb_server::protocol::FieldValueProto::Boolean(false)
            } else {
                tsdb_server::protocol::FieldValueProto::String(val_str.to_string())
            };
            Some((key, value))
        })
        .collect()
}

fn send_request(addr: &str, request: tsdb_server::protocol::Request) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    use tsdb_server::protocol::{encode_request, decode_response};

    let mut stream = std::net::TcpStream::connect(addr)?;

    let data = encode_request(&request);
    let len = data.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&data)?;
    stream.flush()?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let resp_len = u32::from_be_bytes(len_buf) as usize;

    let mut resp_data = vec![0u8; resp_len];
    stream.read_exact(&mut resp_data)?;

    let response = decode_response(&resp_data)
        .ok_or_else(|| anyhow::anyhow!("failed to decode response"))?;

    match response {
        tsdb_server::protocol::Response::Ok => println!("OK"),
        tsdb_server::protocol::Response::Pong => println!("PONG"),
        tsdb_server::protocol::Response::Error(e) => println!("ERROR: {}", e),
        tsdb_server::protocol::Response::Databases(dbs) => {
            for db in dbs {
                println!("  {}", db);
            }
        }
        tsdb_server::protocol::Response::QueryResult { columns, rows } => {
            println!("{}", columns.join("\t"));
            for row in rows {
                let vals: Vec<String> = row.iter().map(|v| format!("{:?}", v)).collect();
                println!("{}", vals.join("\t"));
            }
        }
    }

    Ok(())
}
