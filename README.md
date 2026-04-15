# TSDB — 高性能时序数据库

基于 Rust 构建的高性能时间序列数据库，采用 RocksDB 持久化存储，支持多业务隔离、SQL 查询、向量化执行引擎、MergeOperator 块级合并、数据压缩、NNG 服务接口、仪表盘可视化等企业级功能。

## 架构概览

```
┌──────────────────────────────────────────────────────────────────┐
│                      TSDB Architecture                            │
├──────────┬──────────┬──────────┬──────────┬──────────────────────┤
│  tsdb-cli│tsdb-server           │ tsdb-dashboard                 │
│  (CLI)   │ TCP + HTTP + NNG API │ Business / Performance         │
├──────────┴────┬─────┴───────────┴──────────┬─────────────────────┤
│  tsdb-query   │     tsdb-aggregate         │  tsdb-chart          │
│  SQL Parser   │  Time Dimension Agg        │  SVG/JSON Chart      │
│  Vectorized   │  Hour/Day/Week/Month       │  Line/Area/Bar       │
│  SIMD Engine  │  Async Worker              │                      │
├───────────────┴───────────┬─────────────────┴────────────────────┤
│     tsdb-compress         │    tsdb-index                       │
│  Delta+Zigzag (timestamp) │  SkipList (time index, persistable)  │
│  Gorilla XOR (float)      │  InvertedIndex (tag, persistable)    │
│  Dictionary (string)      │  Roaring Bitmap                     │
├───────────────────────────┴──────────────────────────────────────┤
│              tsdb-core (Storage Engine)                           │
│  RocksDB + ColumnFamily (Hot/Cold Separation)                    │
│  ⭐ MergeOperator: N次访问合并为1次块级读取                        │
│  RowKey: measurement|tags_hash|block_ts (30s blocks)             │
│  BlockBasedTable: 16KB block + TwoLevelIndex + HashIndex         │
│  CF Options: Hot(LZ4+DynamicLevel) / Cold(ZSTD+Universal)       │
├──────────────────────────────────────────────────────────────────┤
│                    tsdb-types (Data Model)                        │
│  DataPoint, FieldValue, Tags, Measurement, SeriesId              │
├───────────────────┬──────────────────────────────────────────────┤
│  tsdb-plugin      │  tsdb-config                                 │
│  Plugin Registry  │  config.ini + Env Override                   │
└───────────────────┴──────────────────────────────────────────────┘
```

## Crate 结构 (12 个模块)

| Crate | 职责 | 测试 |
|-------|------|------|
| **tsdb-types** | 共享数据模型 (DataPoint, FieldValue, Tags) | — |
| **tsdb-core** | 存储引擎 (RocksDB, MergeOperator, RowKey, CF调优, BlockWriter) | 18 ✅ |
| **tsdb-compress** | 压缩算法 (Delta, Gorilla XOR, Dictionary) | 9 ✅ |
| **tsdb-index** | 索引层 (SkipList + 持久化, InvertedIndex + 持久化) | 8 ✅ |
| **tsdb-query** | SQL 解析器 + 查询引擎 + 向量化 SIMD 执行 | 10 ✅ |
| **tsdb-aggregate** | 轻度汇总引擎 (小时/天/周/月维度聚合) | 3 ✅ |
| **tsdb-server** | TCP + HTTP + NNG 三协议服务端 | — |
| **tsdb-cli** | 命令行工具 (start/query/write/ping/list/load-tsbs/generate-tsbs) | — |
| **tsdb-plugin** | 插件系统 (业务/查询/存储插件注册表) | 1 ✅ |
| **tsdb-config** | 配置管理 (config.ini + 环境变量覆盖) | 3 ✅ |
| **tsdb-chart** | 时序图表生成 (SVG 折线/面积/柱状图, JSON) | 4 ✅ |
| **tsdb-dashboard** | 业务仪表盘 + 性能仪表盘 (HTML 渲染) | 3 ✅ |

## 核心特性

### ⭐ MergeOperator — N次访问合并为1次

核心优化：利用 RocksDB MergeOperator 将同一 RowKey 的多个字段合并为一个 MergedBlock，查询时 1 次 get 即可获得全部字段。

| 场景 | 优化前 | 优化后 | 提升 |
|------|--------|--------|------|
| 单点查询 (10字段) | 10次 get/scan | **1次 get_cf** | **10x** |
| 范围查询 (1h, 10metrics) | 3600次迭代 | **360次** | **10x** |
| SST KV 数量 | T×F | **T** | **少F倍** |

### 存储引擎

- **MergeOperator 写入**: `write_merged()` / `write_merged_batch()` — merge_cf 替代 put_cf
- **块级读取**: `get_point_merged()` — 1次 get 获取完整数据点
- **双格式兼容**: 自动检测 MergedBlock/Raw 格式，向后兼容
- **BlockBasedTable 调优**: 16KB block, TwoLevelIndexSearch, BinaryAndHash
- **CF 级别调优**: 热数据 LZ4+DynamicLevel, 冷数据 ZSTD+Universal
- **RowKey 设计**: `{measurement}|{tags_hash}|{block_start_timestamp}`
- **30 秒定长块**: 微秒级精度，优化压缩率和批量查询
- **冷热数据分离**: 按自然日分 CF，秒级数据删除

### 压缩算法

| 数据类型 | 算法 | 描述 |
|----------|------|------|
| 时间戳 | Delta + Zigzag + Varint | 增量编码 + 变长整数 |
| 浮点数 | Gorilla XOR | 异或压缩 + 前导/尾随零优化 |
| 字符串 | Dictionary Encoding | 全局字典 ID 映射 |

### 索引系统 (支持持久化)

- **跳表索引**: 时间范围查询 O(log n)，支持 serialize/deserialize
- **倒排索引**: Tag 精确/交/并集查询，Roaring Bitmap，支持持久化
- **索引管理器**: serialize_all() / deserialize_entry() 完整持久化方案

### NNG 服务接口

- **REQ/REP**: 请求/响应模式 (替代 TCP，端口 9527)
- **PUB/SUB**: 发布/订阅模式 (实时推送，端口 9529)
- **PULL/PUSH**: Pipeline 模式 (高吞吐批量写入，端口 9530)

### 查询引擎

- **SQL 解析**: 基于 sqlparser-rs，支持 SELECT/WHERE/GROUP BY/LIMIT
- **聚合函数**: SUM, AVG, MIN, MAX, COUNT, FIRST, LAST
- **向量化执行**: 列式批处理 (ColumnarBatch)
- **SIMD 优化**: chunk_size=4 批量聚合计算

### 错误处理

- **统一错误类型**: `TsdbError` 替代 `anyhow::Result`
- **18 个变体**: Storage/Compression/Index/Query/Network/Protocol/Nng/Dashboard/NotFound 等
- **From 转换**: serde_json::Error, rocksdb::Error 自动转换

### TSBS 数据验证

- **数据生成**: `generate-tsbs` CLI 子命令 (合成 DevOps 数据集)
- **数据加载**: `load-tsbs` CLI 子命令 (批量导入 + 吞吐量统计)
- **生成脚本**: `scripts/generate_tsbs_data.sh`

### 服务接口

- **TCP 接口**: MessagePack 二进制协议
- **NNG 接口**: REQ/REP + PUB/SUB + PULL/PUSH
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

### 运行测试 (59 个测试)

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

# 生成 TSBS 测试数据
cargo run --bin tsdb-cli -- generate-tsbs --scale 100 --duration 24h --output data.json

# 加载 TSBS 数据
cargo run --bin tsdb-cli -- load-tsbs --input data.json --batch-size 1000
```

## 项目结构

```
tsdb/
├── Cargo.toml                  # Workspace (12 members)
├── config.ini                  # 默认配置
├── .github/workflows/ci.yml    # CI/CD 多平台构建
├── scripts/                    # TSBS 数据生成脚本
├── crates/
│   ├── tsdb-types/             # 数据模型
│   ├── tsdb-core/              # 存储引擎 (RocksDB + MergeOperator + BlockWriter)
│   ├── tsdb-compress/          # 压缩算法
│   ├── tsdb-index/             # 索引层 (可持久化)
│   ├── tsdb-query/             # 查询引擎 + SIMD
│   ├── tsdb-aggregate/         # 汇总引擎
│   ├── tsdb-server/            # TCP + HTTP + NNG 服务
│   ├── tsdb-cli/               # 命令行工具
│   ├── tsdb-plugin/            # 插件系统
│   ├── tsdb-config/            # 配置管理
│   ├── tsdb-chart/             # 图表生成
│   └── tsdb-dashboard/         # 仪表盘渲染
├── frontend/                   # ExpoGo React Native
├── benches/                    # 性能基准测试
├── plan.md                     # 实施状态文档
└── deep-optimize-plan.md       # 深度优化计划文档
```

## 技术栈

- **语言**: Rust 2021 Edition
- **存储**: RocksDB 0.22 (MultiThreaded + MergeOperator + BlockBasedTable调优)
- **消息**: NNG 1.0 (REQ/REP + PUB/SUB + PULL/PUSH)
- **序列化**: serde, MessagePack (rmp-serde), JSON
- **SQL 解析**: sqlparser-rs 0.53
- **压缩**: LZ4 (热数据), ZSTD (冷数据), Gorilla XOR, Delta+Zigzag
- **索引**: SkipList (可持久化), Roaring Bitmap
- **HTTP**: 内置 (无框架依赖)
- **前端**: ExpoGo (React Native)
- **CI/CD**: GitHub Actions (Linux/macOS/Windows, x86_64/ARM64)

## 许可证

MIT License
