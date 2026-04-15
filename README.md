# TSDB — 高性能时序数据库

基于 Rust 构建的高性能时间序列数据库，采用 RocksDB 持久化存储，支持多业务隔离、SQL 查询、向量化执行引擎、数据压缩、仪表盘可视化等企业级功能。

## 架构概览

```
┌─────────────────────────────────────────────────────────────┐
│                      TSDB Architecture                       │
├──────────┬──────────┬──────────┬──────────┬─────────────────┤
│  tsdb-cli│tsdb-server        │ tsdb-dashboard              │
│  (CLI)   │ TCP + HTTP API    │ Business / Performance      │
├──────────┴────┬─────┴─────────┴──────────┬─────────────────┤
│  tsdb-query   │     tsdb-aggregate       │  tsdb-chart      │
│  SQL Parser   │  Time Dimension Agg      │  SVG/JSON Chart  │
│  Vectorized   │  Hour/Day/Week/Month     │  Line/Area/Bar   │
│  SIMD Engine  │  Async Worker            │                  │
├───────────────┴───────────┬───────────────┴─────────────────┤
│     tsdb-compress         │    tsdb-index                 │
│  Delta+Zigzag (timestamp) │  SkipList (time index)         │
│  Gorilla XOR (float)      │  InvertedIndex (tag)          │
│  Dictionary (string)      │  Roaring Bitmap               │
├───────────────────────────┴─────────────────────────────────┤
│              tsdb-core (Storage Engine)                     │
│  RocksDB + ColumnFamily (Hot/Cold Separation)               │
│  RowKey: measurement|tags_hash|block_ts (30s blocks)        │
│  Qualifier: field:microsecond_offset                        │
├─────────────────────────────────────────────────────────────┤
│                    tsdb-types (Data Model)                   │
│  DataPoint, FieldValue, Tags, Measurement, SeriesId         │
├───────────────────┬─────────────────────────────────────────┤
│  tsdb-plugin      │  tsdb-config                            │
│  Plugin Registry  │  config.ini + Env Override              │
└───────────────────┴─────────────────────────────────────────┘
```

## Crate 结构 (12 个模块)

| Crate | 职责 | 测试 |
|-------|------|------|
| **tsdb-types** | 共享数据模型 (DataPoint, FieldValue, Tags) | — |
| **tsdb-core** | 存储引擎 (RocksDB, RowKey, CF管理, 维度表) | 9 ✅ |
| **tsdb-compress** | 压缩算法 (Delta, Gorilla XOR, Dictionary) | 9 ✅ |
| **tsdb-index** | 索引层 (SkipList 时间索引, 倒排标签索引) | 8 ✅ |
| **tsdb-query** | SQL 解析器 + 查询引擎 + 向量化 SIMD 执行 | 10 ✅ |
| **tsdb-aggregate** | 轻度汇总引擎 (小时/天/周/月维度聚合) | 3 ✅ |
| **tsdb-server** | TCP + HTTP 双协议服务端, MessagePack 协议 | — |
| **tsdb-cli** | 命令行工具 (start/query/write/ping/list) | — |
| **tsdb-plugin** | 插件系统 (业务/查询/存储插件注册表) | 1 ✅ |
| **tsdb-config** | 配置管理 (config.ini + 环境变量覆盖) | 3 ✅ |
| **tsdb-chart** | 时序图表生成 (SVG 折线/面积/柱状图, JSON) | 4 ✅ |
| **tsdb-dashboard** | 业务仪表盘 + 性能仪表盘 (HTML 渲染) | 3 ✅ |

## 核心特性

### 存储引擎

- **RowKey 设计**: `{measurement}|{tags_hash}|{block_start_timestamp}`
- **Qualifier 设计**: `{field_name}:{microsecond_offset}`
- **30 秒定长块**: 优化压缩率和批量查询性能
- **微秒级精度**: 满足高频交易/IoT 场景需求
- **冷热数据分离**: 按自然日分 ColumnFamily，热数据 LZ4，冷数据 ZSTD
- **秒级数据删除**: 直接 Drop 过期 CF，无 Compaction 抖动

### 压缩算法

| 数据类型 | 算法 | 描述 |
|----------|------|------|
| 时间戳 | Delta + Zigzag + Varint | 增量编码 + 变长整数 |
| 浮点数 | Gorilla XOR | 异或压缩 + 前导/尾随零优化 |
| 字符串 | Dictionary Encoding | 全局字典 ID 映射 |

### 索引系统

- **跳表索引**: 时间范围查询 O(log n)
- **倒排索引**: Tag 精确/交/并集查询，Roaring Bitmap 压缩位图
- **索引管理器**: 统一的时间索引 + 标签索引协调

### 查询引擎

- **SQL 解析**: 基于 sqlparser-rs，支持 SELECT/WHERE/GROUP BY/LIMIT
- **聚合函数**: SUM, AVG, MIN, MAX, COUNT, FIRST, LAST
- **向量化执行**: 列式批处理 (ColumnarBatch)
- **SIMD 优化**: chunk_size=4 批量聚合计算

### 汇总引擎

- **时间维度**: 小时 / 天 / 周 / 月
- **异步计算**: 写入时触发异步汇总任务
- **独立存储**: 汇总数据单独 RocksDB 实例，长期保存不修改
- **Worker 模型**: 多 Worker 轮询分发，缓冲区批量刷写

### 仪表盘系统

- **业务仪表盘**: 指标卡片、趋势箭头(↑↓→)、变化百分比
- **性能仪表盘**: 仪表盘(Good/Warning/Critical 三级)、系统指标、历史记录
- **HTML 渲染**: 完整可交互的 Dashboard HTML 页面输出
- **图表集成**: 与 tsdb-chart 联动生成可视化图表

### 插件机制

```rust
// 业务插件
pub trait BusinessPlugin: Send + Sync { ... }
// 查询插件
pub trait QueryPlugin: Send + Sync { ... }
// 存储插件
pub trait StoragePlugin: Send + Sync { ... }
```

### 服务接口

- **TCP 接口**: MessagePack 二进制协议
- **HTTP RESTful API**:
  - `GET /api/v1/ping` — 健康检查
  - `POST /api/v1/write` — 写入数据点
  - `POST /api/v1/query` — SQL 查询
  - `GET /api/v1/chart?sql=...` — 图表生成 (SVG)
  - `GET /api/v1/dashboard/business?sql=...` — 业务仪表盘 (HTML)
  - `GET /api/v1/dashboard/performance` — 性能仪表盘 (HTML)

## 快速开始

### 编译

```bash
cargo build --release
```

### 运行测试 (50 个测试)

```bash
cargo test --lib
```

### 启动服务端

```bash
cargo run --bin tsdb-cli -- start
```

### CLI 使用

```bash
# 查询数据
cargo run --bin tsdb-cli -- query "SELECT * FROM cpu WHERE host='server-1'"

# 写入数据
cargo run --bin tsdb-cli -- write --measurement cpu --tags host=server-1 --fields usage=0.75

# Ping 服务
cargo run --bin tsdb-cli -- ping

# 列出数据库
cargo run --bin tsdb-cli -- list
```

### 配置文件 (config.ini)

```ini
[server]
host = 0.0.0.0
port = 9527
http_port = 9528

[storage]
data_dir = ./data
block_duration_secs = 30
hot_cf_compression = lz4
cold_cf_compression = zstd

[aggregate]
enabled = true
workers = 4
buffer_size = 1024
```

环境变量覆盖: `TSDB_HOST`, `TSDB_PORT`, `TSDB_HTTP_PORT`, `TSDB_DATA_DIR` 等。

## 项目结构

```
tsdb/
├── Cargo.toml                  # Workspace (12 members)
├── config.ini                  # 默认配置
├── .github/workflows/ci.yml    # CI/CD 多平台构建
├── crates/
│   ├── tsdb-types/             # 数据模型
│   ├── tsdb-core/              # 存储引擎 (RocksDB)
│   ├── tsdb-compress/          # 压缩算法
│   ├── tsdb-index/             # 索引层
│   ├── tsdb-query/             # 查询引擎 + SIMD
│   ├── tsdb-aggregate/         # 汇总引擎
│   ├── tsdb-server/            # TCP + HTTP 服务
│   ├── tsdb-cli/               # 命令行工具
│   ├── tsdb-plugin/            # 插件系统
│   ├── tsdb-config/            # 配置管理
│   ├── tsdb-chart/             # 图表生成
│   └── tsdb-dashboard/         # 仪表盘渲染
├── frontend/                   # ExpoGo React Native
├── benches/                    # 性能基准测试
└── plan.md                     # 实施状态文档
```

## 技术栈

- **语言**: Rust 2021 Edition
- **存储**: RocksDB 0.22 (MultiThreaded mode)
- **序列化**: serde, MessagePack (rmp-serde), JSON
- **SQL 解析**: sqlparser-rs 0.53
- **压缩**: LZ4 (热数据), ZSTD (冷数据), Gorilla XOR, Delta+Zigzag
- **索引**: SkipList, Roaring Bitmap
- **HTTP**: 内置 (无框架依赖)
- **前端**: ExpoGo (React Native)
- **CI/CD**: GitHub Actions (Linux/macOS/Windows, x86_64/ARM64)

## 许可证

MIT License
