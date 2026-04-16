//! TSDB 命令行工具 - TSDB Command Line Interface
//!
//! 本模块提供 TSDB 的命令行交互界面，支持以下操作：
//! - `start`: 启动 TSDB 服务端
//! - `query`: 执行 SQL 查询
//! - `write`: 写入数据点
//! - `ping`: 健康检查
//! - `list`: 列出数据库
//! - `load-tsbs`: 加载 TSBS 基准测试数据
//! - `generate-tsbs`: 生成 TSBS 合成数据
//!
//! ## 使用示例
//!
//! ```bash
//! # 启动服务端
//! tsdb-cli start --config config.ini
//!
//! # 执行查询
//! tsdb-cli --host 127.0.0.1 --port 7878 query "SELECT * FROM cpu"
//!
//! # 写入数据
//! tsdb-cli write --measurement cpu --tags host=server01 --fields usage=0.75
//!
//! # 健康检查
//! tsdb-cli ping
//!
//! # 加载 TSBS 数据
//! tsdb-cli load-tsbs --input data.json --batch-size 1000
//! ```

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tsdb_config::TsdbConfig;

/// TSDB 命令行客户端
///
/// 使用 `clap` 库解析命令行参数，支持子命令模式。
#[derive(Parser)]
#[command(
    name = "tsdb-cli",
    version = "0.1.0",
    about = "TSDB command line client"
)]
struct Cli {
    /// 子命令
    #[command(subcommand)]
    command: Commands,

    /// 服务端主机地址
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// 服务端端口号
    #[arg(long, default_value_t = 7878)]
    port: u16,
    /// 目标数据库名称（默认 "default"）
    #[arg(long, default_value = "")]
    database: String,
}

/// 子命令枚举
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
    /// 创建新数据库
    CreateDb {
        name: String,
    },
    /// 删除数据库
    DropDb {
        name: String,
    },
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

/// 程序入口函数
fn main() -> anyhow::Result<()> {
    // 解析命令行参数
    let cli = Cli::parse();

    // 根据子命令执行相应操作
    match cli.command {
        Commands::Start { config } => {
            let config = TsdbConfig::load_or_default(&config);
            let mut server = tsdb_server::TsdbServer::new(config);
            server.start()?;
        }
        Commands::Query { sql } => {
            let addr = format!("{}:{}", cli.host, cli.port);
            send_request(
                &addr,
                tsdb_server::protocol::Request::Query {
                    database: cli.database.clone(),
                    sql,
                },
            )?;
        }
        Commands::Write {
            measurement,
            tags,
            fields,
            timestamp,
        } => {
            let tag_pairs = parse_kv_pairs(tags.as_deref());
            let field_pairs = parse_field_values(&fields);
            let ts = if timestamp == 0 {
                chrono::Utc::now().timestamp_micros()
            } else {
                timestamp
            };
            let addr = format!("{}:{}", cli.host, cli.port);
            send_request(
                &addr,
                tsdb_server::protocol::Request::Write {
                    database: cli.database.clone(),
                    measurement,
                    tags: tag_pairs,
                    fields: field_pairs,
                    timestamp: ts,
                },
            )?;
        }
        Commands::Ping => {
            let addr = format!("{}:{}", cli.host, cli.port);
            send_request(&addr, tsdb_server::protocol::Request::Ping)?;
        }
        Commands::List => {
            let addr = format!("{}:{}", cli.host, cli.port);
            send_request(&addr, tsdb_server::protocol::Request::ListDatabases)?;
        }
        Commands::CreateDb { name } => {
            let addr = format!("{}:{}", cli.host, cli.port);
            send_request(
                &addr,
                tsdb_server::protocol::Request::CreateDatabase { name },
            )?;
        }
        Commands::DropDb { name } => {
            let addr = format!("{}:{}", cli.host, cli.port);
            send_request(&addr, tsdb_server::protocol::Request::DropDatabase { name })?;
        }
        Commands::LoadTsbs { input, batch_size } => {
            load_tsbs_data(&input, batch_size)?;
        }
        Commands::GenerateTsbs {
            scale,
            duration,
            output,
        } => {
            generate_tsbs_data(scale, &duration, &output)?;
        }
    }

    Ok(())
}

/// 加载 TSBS 基准测试数据
///
/// 从 JSON 文件读取 TSBS 格式的数据并批量写入服务端。
/// 支持进度显示和吞吐量统计。
///
/// # 参数
///
/// - `path`: 输入文件路径
/// - `batch_size`: 批量写入大小
///
/// # 返回值
///
/// 成功返回 `Ok(())`，失败返回错误
///
/// # 数据格式
///
/// 每行一个 JSON 对象：
/// ```json
/// {
///   "measurement": "cpu",
///   "timestamp": "2025-01-01T00:00:00Z",
///   "tags": {"host": "server01"},
///   "fields": {"usage": 0.75}
/// }
/// ```
fn load_tsbs_data(path: &std::path::Path, batch_size: usize) -> anyhow::Result<()> {
    use std::io::BufRead;
    use tsdb_server::protocol::{FieldValueProto, Request};

    // 打开文件并创建缓冲读取器
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let addr = "127.0.0.1:7878".to_string();

    let mut total = 0u64;
    let mut batch = Vec::new();
    let start = std::time::Instant::now();

    // 逐行读取并解析
    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // 解析 JSON
        let json: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // 提取字段
        let measurement = json["measurement"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let timestamp = json["timestamp"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp_micros())
            .unwrap_or_else(|| chrono::Utc::now().timestamp_micros());

        // 解析标签
        let mut tags = Vec::new();
        if let Some(tags_obj) = json["tags"].as_object() {
            for (k, v) in tags_obj {
                if let Some(s) = v.as_str() {
                    tags.push((k.clone(), s.to_string()));
                }
            }
        }

        // 解析字段值
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

        // 添加到批次
        batch.push(Request::Write {
            database: String::new(),
            measurement,
            tags,
            fields,
            timestamp,
        });

        // 批量发送
        if batch.len() >= batch_size {
            for req in batch.drain(..) {
                if let Ok(()) = send_request_silent(&addr, req) {
                    total += 1;
                }
            }
            // 显示进度
            if total.is_multiple_of(10000) {
                let elapsed = start.elapsed().as_secs_f64();
                let throughput = total as f64 / elapsed;
                eprint!("\rLoaded {} points ({:.0} pts/sec)", total, throughput);
            }
        }
    }

    // 发送剩余数据
    for req in batch.drain(..) {
        if let Ok(()) = send_request_silent(&addr, req) {
            total += 1;
        }
    }

    // 显示最终统计
    let elapsed = start.elapsed().as_secs_f64();
    let throughput = total as f64 / elapsed;
    println!(
        "\nLoaded {} points in {:.2}s ({:.0} pts/sec)",
        total, elapsed, throughput
    );

    Ok(())
}

/// 静默发送请求（不显示响应）
///
/// 用于批量操作时减少输出干扰。
fn send_request_silent(addr: &str, request: tsdb_server::protocol::Request) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    use tsdb_server::protocol::encode_request;

    let mut stream = std::net::TcpStream::connect(addr)?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;

    // 编码并发送请求
    let data = encode_request(&request);
    let len = data.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&data)?;
    stream.flush()?;

    // 读取响应（但不处理）
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let resp_len = u32::from_be_bytes(len_buf) as usize;
    let mut resp_data = vec![0u8; resp_len];
    stream.read_exact(&mut resp_data)?;

    Ok(())
}

/// 生成 TSBS 合成数据
///
/// 生成模拟的 DevOps CPU 监控数据，用于性能测试。
///
/// # 参数
///
/// - `scale`: 设备数量
/// - `duration`: 数据持续时间
/// - `output`: 输出文件路径
///
/// # 数据模型
///
/// 每个设备生成一条 CPU 时间序列，包含以下字段：
/// - usage_user: 用户态 CPU 使用率
/// - usage_system: 内核态 CPU 使用率
/// - usage_idle: 空闲 CPU 比例
/// - usage_nice: nice 值 CPU 使用率
/// - usage_iowait: I/O 等待 CPU 比例
fn generate_tsbs_data(
    scale: usize,
    duration: &str,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    // 解析持续时间
    let duration_secs = parse_duration(duration)?;
    let interval_secs: u64 = 10; // 10 秒采集间隔
    let points_per_device = (duration_secs / interval_secs) as usize;
    let total_points = scale * points_per_device;

    // 显示生成参数
    println!("Generating synthetic TSBS data:");
    println!("  Devices: {}", scale);
    println!("  Duration: {}s", duration_secs);
    println!("  Interval: {}s", interval_secs);
    println!("  Total points: {}", total_points);

    // 创建输出文件
    let mut file: Box<dyn std::io::Write> =
        Box::new(std::io::BufWriter::new(std::fs::File::create(output)?));
    let base_ts = chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")?.timestamp_micros();

    let mut count = 0u64;
    // 遍历每个设备
    for device_id in 0..scale {
        let hostname = format!("host_{}", device_id);
        // 遍历每个时间点
        for point_idx in 0..points_per_device {
            let ts = base_ts + (point_idx as i64 * interval_secs as i64 * 1_000_000);

            // 构造 JSON 数据点
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
                    // 模拟 CPU 使用率数据
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

        // 显示进度
        if device_id % 100 == 0 {
            eprint!(
                "\rGenerated {}/{} devices ({:.0}%)",
                device_id,
                scale,
                device_id as f64 / scale as f64 * 100.0
            );
        }
    }

    println!("\nGenerated {} points to {:?}", count, output);
    Ok(())
}

/// 解析持续时间字符串
///
/// 支持以下格式：
/// - `h`: 小时
/// - `d`: 天
/// - `m`: 分钟
/// - `s`: 秒
/// - 纯数字: 秒
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

/// 解析键值对字符串
///
/// 格式：`key1=value1,key2=value2,...`
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

/// 解析字段值字符串
///
/// 自动推断值类型：
/// - 浮点数：包含小数点的数字
/// - 整数：纯数字
/// - 布尔：`true` 或 `false`
/// - 字符串：其他情况
fn parse_field_values(input: &str) -> Vec<(String, tsdb_server::protocol::FieldValueProto)> {
    input
        .split(',')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next()?.to_string();
            let val_str = parts.next()?;
            // 自动推断类型
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

/// 发送请求并显示响应
///
/// 通过 TCP 连接发送请求，并打印服务端响应。
fn send_request(addr: &str, request: tsdb_server::protocol::Request) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    use tsdb_server::protocol::{decode_response, encode_request};

    let mut stream = std::net::TcpStream::connect(addr)?;

    // 编码并发送请求
    let data = encode_request(&request);
    let len = data.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&data)?;
    stream.flush()?;

    // 读取响应长度
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let resp_len = u32::from_be_bytes(len_buf) as usize;

    // 读取响应数据
    let mut resp_data = vec![0u8; resp_len];
    stream.read_exact(&mut resp_data)?;

    // 解码并显示响应
    let response =
        decode_response(&resp_data).ok_or_else(|| anyhow::anyhow!("failed to decode response"))?;

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
            // 显示列名
            println!("{}", columns.join("\t"));
            // 显示数据行
            for row in rows {
                let vals: Vec<String> = row
                    .iter()
                    .map(|v: &tsdb_server::protocol::FieldValueProto| format!("{:?}", v))
                    .collect();
                println!("{}", vals.join("\t"));
            }
        }
    }

    Ok(())
}
