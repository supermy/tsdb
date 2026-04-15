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
    LoadTsbs {
        #[arg(long)]
        input: PathBuf,
        #[arg(long, default_value_t = 1000)]
        batch_size: usize,
    },
    GenerateTsbs {
        #[arg(long, default_value_t = 100)]
        scale: usize,
        #[arg(long, default_value = "24h")]
        duration: String,
        #[arg(long, default_value = "tsbs_data.json")]
        output: PathBuf,
    },
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
        Commands::LoadTsbs { input, batch_size } => {
            load_tsbs_data(&input, batch_size)?;
        }
        Commands::GenerateTsbs { scale, duration, output } => {
            generate_tsbs_data(scale, &duration, &output)?;
        }
    }

    Ok(())
}

fn load_tsbs_data(path: &std::path::Path, batch_size: usize) -> anyhow::Result<()> {
    use std::io::BufRead;
    use tsdb_server::protocol::{Request, FieldValueProto};

    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let addr = format!("127.0.0.1:7878");

    let mut total = 0u64;
    let mut batch = Vec::new();
    let start = std::time::Instant::now();

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() { continue; }

        let json: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let measurement = json["measurement"].as_str().unwrap_or("unknown").to_string();
        let timestamp = json["timestamp"].as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp_micros())
            .unwrap_or_else(|| chrono::Utc::now().timestamp_micros());

        let mut tags = Vec::new();
        if let Some(tags_obj) = json["tags"].as_object() {
            for (k, v) in tags_obj {
                if let Some(s) = v.as_str() {
                    tags.push((k.clone(), s.to_string()));
                }
            }
        }

        let mut fields = Vec::new();
        if let Some(fields_obj) = json["fields"].as_object() {
            for (k, v) in fields_obj {
                let fv = if let Some(f) = v.as_f64() {
                    FieldValueProto::Float(f)
                } else if let Some(i) = v.as_i64() {
                    FieldValueProto::Integer(i)
                } else if let Some(s) = v.as_str() {
                    FieldValueProto::String(s.to_string())
                } else if let Some(b) = v.as_bool() {
                    FieldValueProto::Boolean(b)
                } else {
                    continue;
                };
                fields.push((k.clone(), fv));
            }
        }

        batch.push(Request::Write { measurement, tags, fields, timestamp });

        if batch.len() >= batch_size {
            for req in batch.drain(..) {
                if let Ok(()) = send_request_silent(&addr, req) {
                    total += 1;
                }
            }
            if total % 10000 == 0 {
                let elapsed = start.elapsed().as_secs_f64();
                let throughput = total as f64 / elapsed;
                eprint!("\rLoaded {} points ({:.0} pts/sec)", total, throughput);
            }
        }
    }

    for req in batch.drain(..) {
        if let Ok(()) = send_request_silent(&addr, req) {
            total += 1;
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    let throughput = total as f64 / elapsed;
    println!("\nLoaded {} points in {:.2}s ({:.0} pts/sec)", total, elapsed, throughput);

    Ok(())
}

fn send_request_silent(addr: &str, request: tsdb_server::protocol::Request) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    use tsdb_server::protocol::{encode_request, decode_response};

    let mut stream = std::net::TcpStream::connect(addr)?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;

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

    Ok(())
}

fn generate_tsbs_data(scale: usize, duration: &str, output: &std::path::Path) -> anyhow::Result<()> {
    let duration_secs = parse_duration(duration)?;
    let interval_secs: u64 = 10;
    let points_per_device = (duration_secs / interval_secs) as usize;
    let total_points = scale * points_per_device;

    println!("Generating synthetic TSBS data:");
    println!("  Devices: {}", scale);
    println!("  Duration: {}s", duration_secs);
    println!("  Interval: {}s", interval_secs);
    println!("  Total points: {}", total_points);

    let mut file: Box<dyn std::io::Write> = Box::new(std::io::BufWriter::new(std::fs::File::create(output)?));
    let base_ts = chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")?.timestamp_micros();

    let mut count = 0u64;
    for device_id in 0..scale {
        let hostname = format!("host_{}", device_id);
        for point_idx in 0..points_per_device {
            let ts = base_ts + (point_idx as i64 * interval_secs as i64 * 1_000_000);

            let json = serde_json::json!({
                "measurement": "cpu",
                "timestamp": chrono::DateTime::from_timestamp(ts / 1_000_000, 0)
                    .map(|dt| dt.format("%+").to_string())
                    .unwrap_or_default(),
                "tags": {
                    "hostname": hostname,
                    "region": format!("region_{}", device_id % 10),
                    "datacenter": format!("dc_{}", device_id % 5),
                },
                "fields": {
                    "usage_user": 50.0 + (device_id as f64 * 0.1 + point_idx as f64 * 0.01) % 50.0,
                    "usage_system": 20.0 + (device_id as f64 * 0.05) % 30.0,
                    "usage_idle": 100.0 - (50.0 + (device_id as f64 * 0.1) % 50.0) - (20.0 + (device_id as f64 * 0.05) % 30.0),
                    "usage_nice": 1.0 + (device_id as f64 * 0.01) % 5.0,
                    "usage_iowait": 2.0 + (point_idx as f64 * 0.02) % 10.0,
                }
            });

            writeln!(file, "{}", json)?;
            count += 1;
        }

        if device_id % 100 == 0 {
            eprint!("\rGenerated {}/{} devices ({:.0}%)", device_id, scale, device_id as f64 / scale as f64 * 100.0);
        }
    }

    println!("\nGenerated {} points to {:?}", count, output);
    Ok(())
}

fn parse_duration(s: &str) -> anyhow::Result<u64> {
    let s = s.trim();
    if s.ends_with('h') {
        let hours: u64 = s.trim_end_matches('h').parse()?;
        Ok(hours * 3600)
    } else if s.ends_with('d') {
        let days: u64 = s.trim_end_matches('d').parse()?;
        Ok(days * 86400)
    } else if s.ends_with('m') {
        let mins: u64 = s.trim_end_matches('m').parse()?;
        Ok(mins * 60)
    } else if s.ends_with('s') {
        Ok(s.trim_end_matches('s').parse()?)
    } else {
        Ok(s.parse()?)
    }
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
