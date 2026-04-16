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
| **tsdb-types** | 共享数据模型 (DataPoint, FieldValue, Tags) | 1 ✅ |
| **tsdb-core** | 存储引擎 (RocksDB, MergeOperator, RowKey, CF调优, BlockWriter) | 18 ✅ |
| **tsdb-compress** | 压缩算法 (Delta, Gorilla XOR, Dictionary) | 17 ✅ |
| **tsdb-index** | 索引层 (SkipList + 持久化, InvertedIndex + 持久化) | 37 ✅ |
| **tsdb-query** | SQL 解析器 + 查询引擎 + 向量化 SIMD 执行 | 16 ✅ |
| **tsdb-aggregate** | 轻度汇总引擎 (小时/天/周/月维度聚合) | 22 ✅ |
| **tsdb-server** | TCP + HTTP + NNG 三协议服务端 | 16 ✅ |
| **tsdb-cli** | 命令行工具 (start/query/write/ping/list/load-tsbs/generate-tsbs) | — |
| **tsdb-plugin** | 插件系统 (业务/查询/存储插件注册表) | 1 ✅ |
| **tsdb-config** | 配置管理 (config.ini + 环境变量覆盖) | 3 ✅ |
| **tsdb-chart** | 时序图表生成 (SVG 折线/面积/柱状图, JSON) | 4 ✅ |
| **tsdb-dashboard** | 业务仪表盘 + 性能仪表盘 (HTML 渲染) | 1 ✅ |

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
| 时间戳 | Delta-of-Delta + RLE + Zigzag + Varint | 增量编码 + RLE + 变长整数 |
| 浮点数 | Gorilla XOR | 异或压缩 + 前导/尾随零优化 |
| 整数 | Simple8b + Zigzag | 64-bit 字对齐整数压缩 |
| 字符串 | Dictionary Encoding | 全局字典 ID 映射 |
| 布尔 | Bit-packing | 位打包压缩 |

### 索引系统 (支持持久化)

- **跳表索引**: 时间范围查询 O(log n)，支持 serialize/deserialize
- **倒排索引**: Tag 精确/交/并集查询，Roaring Bitmap，支持持久化
- **布隆过滤器**: Series Key 快速排除，1% 误判率，查询前预检
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
  - `GET /health` — 健康检查
  - `POST /api/v1/write` — 写入数据点
  - `GET /api/v1/query?sql=...` — SQL 查询
  - `GET/POST /api/v1/timeseries` — 时序数据查询
  - `POST /api/v1/databases/{name}` — 创建数据库
  - `DELETE /api/v1/databases/{name}` — 删除数据库
  - `GET /api/v1/databases` — 列出所有数据库

## 快速开始

### 编译

```bash
cargo build --release
```

### 运行测试 (168 个测试)

```bash
cargo test --all
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
├── deep-optimize-plan.md       # 深度优化计划文档
├── tdd-implementation-plan.md  # TDD 实施计划文档
└── docs/
    └── architecture-diagrams.md # 架构可视化文档 (Mermaid)
```

## 技术栈

- **语言**: Rust 2021 Edition
- **存储**: RocksDB 0.22 (MultiThreaded + MergeOperator + BlockBasedTable调优)
- **消息**: NNG 1.0 (REQ/REP + PUB/SUB + PULL/PUSH)
- **序列化**: serde, MessagePack (rmp-serde), JSON
- **SQL 解析**: sqlparser-rs 0.53
- **压缩**: LZ4 (热数据), ZSTD (冷数据), Gorilla XOR, Delta+Zigzag
- **索引**: SkipList (可持久化), Roaring Bitmap
- **HTTP**: warp (异步 HTTP 框架)
- **前端**: ExpoGo (React Native)
- **CI/CD**: GitHub Actions (Linux/macOS/Windows, x86_64/ARM64)

## 许可证

MIT License

---

## 性能基准测试

### 测试环境

- **平台**: macOS (Apple Silicon M1), 16GB RAM
- **编译**: `cargo run --release`
- **数据集**: TSBS DevOps (CPU/Memory/Disk/Network × 1000 主机)

### 写入 QPS

| 写入模式 | 优化前 QPS | 优化后 QPS | 提升 | 说明 |
|---------|-----------|-----------|------|------|
| 单条写入 `write()` | 6,321 | **18,631** | **2.9x** | WAL 异步 + fdatasync |
| 批量写入 `write_batch()` | 45,207 | 39,815 | — | WAL 异步影响批量模式 |
| MergeBlock 写入 `write_merged()` | 9,609 | **33,231** | **3.5x** | merge_cf 延迟合并 |
| 大规模批量 (1.44M 点) | 36,761 | **43,545** | **1.2x** | WAL 异步 + Group Commit |

### 查询延迟

| 查询类型 | 延迟 | 说明 |
|---------|------|------|
| 单点查询 `get_point_merged()` | ~0.15ms | MergedBlock 1次 get |
| 范围查询 (1小时) | ~2.5ms | 前缀迭代 + MergedBlock 解码 |
| Series Key Bloom Filter | ~0.001ms | 布隆过滤器预检，false=跳过 |

### 压缩效率

| 压缩算法 | 优化前压缩比 | 优化后压缩比 | 提升 | 说明 |
|---------|------------|------------|------|------|
| Delta (时间戳) | 8.0:1 | **5,000:1** | **625x** | Delta-of-Delta + RLE |
| Gorilla (浮点) | ~5:1 | ~1.1:1 | — | 随机浮点压缩有限 |
| Dictionary (字符串) | 1,014:1 | **1,014:1** | — | 字典编码不变 |
| Simple8b (整数) | 1:1 | **7.5:1** | **7.5x** | 新增 Simple8b + Zigzag |
| BlockCodec 整体 | 24.0:1 | **15,000:1** | **625x** | RLE + Simple8b 协同 |

### 聚合管道

| 指标 | 数值 | 说明 |
|------|------|------|
| 聚合吞吐 | 106,054 pts/sec | LightAggregationPipeline 内存缓冲 |
| 多 DB 隔离开销 | 0.9% | MultiDbManager vs 单 DB |

### 运行基准测试

```bash
cargo run --release --bin tsdb-bench
```

---

## 对标 InfluxDB 技术分析

### 架构对比

| 维度 | 本 TSDB | InfluxDB TSM (v1/v2) | InfluxDB 3.0 (Arrow) |
|------|---------|---------------------|---------------------|
| **语言** | Rust | Go | Rust |
| **存储引擎** | RocksDB (LSM-Tree) | 自研 TSM (LSM 变体) | Apache Arrow + Parquet |
| **写入模型** | WAL → MemTable → SST | WAL → Cache → TSM File | WAL → Arrow Buffer → Parquet |
| **压缩** | Gorilla/Delta/Dict | Gorilla/Snappy/Delta | ZSTD + Gorilla |
| **索引** | SkipList + InvertedIndex | 内存倒排索引 | DataFusion + Parquet 索引 |
| **查询引擎** | SQL Parser + Vectorized | InfluxQL + Flux | DataFusion SQL |
| **列式存储** | MergedBlock (逻辑列) | TSM 列式文件 | Arrow 列式内存 |
| **聚合** | Pipeline + Aggregator | Continuous Query | Materialized View |

### 性能差距分析

| 指标 | 本 TSDB | InfluxDB TSM | 差距原因 | 优化空间 |
|------|---------|-------------|---------|---------|
| 单线程写入 | 6K-9K | 100K+ | 每次写入 CF 查找 + 编码开销 | ⭐ 写入批处理优化 |
| 批量写入 | 36K-45K | 200K+ | RocksDB WriteBatch 延迟 | ⭐ WAL 异步 + Group Commit |
| 压缩比 (float) | 5:1 | 10-15:1 | 未利用时间局部性 | ⭐ 块级 Gorilla 精度优化 |
| 压缩比 (ts) | 8:1 | 15-20:1 | Delta-of-Delta 未完全实现 | ⭐ 完整 DoD + RLE |
| 查询延迟 | 2.5ms | 1-5ms | 相当 | ✅ 已达标 |
| 聚合吞吐 | 111K | 500K+ | 内存结构效率 | ⭐ HashMap → 列式累加 |

### 可落地的 InfluxDB 对标优化

#### P0: 写入路径优化 (✅ 已实施)

| # | 优化项 | InfluxDB 做法 | 实施方案 | 效果 |
|---|--------|-------------|---------|------|
| W1 | **WriteBatch Group Commit** | TSM Cache 批量刷盘 | AsyncBlockWriter 后台 flush 线程 | ✅ 1.2x 大规模写入 |
| W2 | **WAL 异步 fsync** | 异步 WAL + 定期 sync | set_use_fsync(false) + manual_wal_flush | ✅ 2.9x 单条写入 |
| W3 | **Series Cache 延迟 merge** | Cache 层聚合同 series | BlockWriter flush_block → merge_cf | ✅ 3.5x MergeBlock |

#### P1: 压缩算法优化 (✅ 已实施)

| # | 优化项 | InfluxDB 做法 | 实施方案 | 效果 |
|---|--------|-------------|---------|------|
| C1 | **Delta-of-Delta + RLE** | DoD + RLE + Varint | RLE marker(0xFF) + repeat_count | ✅ 5000:1 时间戳压缩 |
| C2 | **Gorilla 精度优化** | 双精度 XOR + 前导零分组 | 已有标准 Gorilla 实现 | ✅ 基本达标 |
| C3 | **Simple8b 编码** | Simple8b + RLE 混合 | 16 selector 字对齐 + Zigzag | ✅ 7.5:1 整数压缩 |

#### P2: 查询引擎优化 (部分实施)

| # | 优化项 | InfluxDB 做法 | 实施方案 | 效果 |
|---|--------|-------------|---------|------|
| Q1 | **Bloom Filter Series Key** | Series Key 布隆过滤 | BloomFilter 1% FPP + merge | ✅ 快速排除 |
| Q2 | **列式内存布局** | Arrow 列式零拷贝 | ColumnarBatch Vec<T> | ✅ 已实现 |
| Q3 | **Parquet 存储** | Parquet 列式文件 | — | 🔜 长期目标 |

#### P3: InfluxDB 3.0 方向 (长期)

| # | 优化项 | 说明 |
|---|--------|------|
| A1 | **Apache Arrow 内存模型** | 零拷贝列式处理，消除序列化开销 |
| A2 | **DataFusion 查询引擎** | 替代自研 SQL Parser，支持复杂查询 |
| A3 | **Parquet 持久化** | 列式文件存储，比 SST 更适合分析查询 |
| A4 | **Object Storage 分层** | 热数据 SSD + 冷数据 S3，降低存储成本 |

### 优化优先级路线图

```
优化前 (6K single write, 8:1 ts compression, 1:1 int compression)
  │
  ├── ✅ W2: WAL 异步 fsync                    →  18.6K single write (2.9x)
  ├── ✅ W3: Series Cache 延迟 merge            →  33.2K merged write (3.5x)
  ├── ✅ W1: AsyncBlockWriter Group Commit       →  43.5K batch write (1.2x)
  │
  ├── ✅ C1: Delta-of-Delta + RLE               →  5000:1 ts compression
  ├── ✅ C3: Simple8b 整数编码                   →  7.5:1 int compression
  │
  ├── ✅ Q1: Bloom Filter Series Key             →  快速排除不存在的 series
  │
  └── 🔜 A1+A2: Arrow + DataFusion (InfluxDB 3.0 方向) →  500K+ write, 10ms query
```

---

## 优化实施记录

### Round 1 (commit d3101e0)
- CompressError → TsdbError From impl
- QueryEngine 聚合默认分支修复
- StorageEngine write() 预分配 key buffer
- IndexWAL CRC mismatch 日志增强

### Round 2 (commit 5787138)
- WAL append_entry: &self → &mut self, 写入走 BufWriter (修复绕过 bug)
- WAL rotate(): 实际执行 flush+rename+reopen (修复空操作 bug)
- read_range prefix_key 移到循环外
- Aggregator finalize 引用迭代
- Worker finalize 动态发现 measurement
- HTTP API 描述性 panic
- Protocol encode .expect() 替代 .unwrap_or_default()
- 移除未使用的 byteorder 依赖

### Round 3 (commit 2315488)
- Qualifier::new 整数截断保护 (assert 范围检查)
- block_writer / merge_operand 截断保护
- TOCTOU race 修复 (create_database/ensure_default/get_store 原子写锁)
- SeriesId u64→u32 溢出检查
- RwLock/Mutex unwrap() → unwrap_or_else (锁中毒恢复)
- SkipList O(n²) → O(n) 反序列化 (HashMap 索引)
- InvertedIndex query_intersection 原地 &= (避免 N 次 clone)
- PerformanceDashboard Vec::remove(0) → VecDeque::pop_front()
- SQL Parser 不支持的操作符返回错误
- Comparator 比较 Qualifier 部分 (修复不同键判为相等)
- InvertedIndex serialize 传播错误
- DimensionTable O(n) → O(1) 反向映射

### 测试补充 (commit f6623ea)
- 新增 33 个单元测试，总计 148 个
- 覆盖: Protocol(16), Parser(7), Aggregator(6), Codec(5), Dictionary(4), InvertedIndex(4), Pipeline(2)

### 功能完善 (commit 50f1de6)
- HTTP API: CreateDB/DropDB/ListDB 路由
- IndexScan: 利用 tag 过滤做精确 read_range
- README 更新: 测试数、API 端点、文档结构
- TDD 实施计划文档

### Bug 修复 (commit 0acd0c4)
- 修复 write_merged() 崩溃: hot/cold CF 选项未注册 merge_operator
- 基准测试全部通过，真实 QPS 数据入库

### InfluxDB 对标优化 Round 4 (168 测试)
- **H1.2 WAL 异步 fsync**: set_use_fsync(false) + manual_wal_flush + PointInTime 恢复模式
- **H2.3 Simple8b 整数编码**: 16 selector 字对齐压缩 + Zigzag 编码，整数压缩从 1:1 → 7.5:1
- **H2.1 RLE 编码**: Delta-of-Delta + RLE，固定间隔时间戳压缩从 8:1 → 5000:1
- **H3.1 Series Key Bloom Filter**: 1% FPP 布隆过滤器，快速排除不存在的 series
- **H1.3 Series Cache 延迟 merge**: BlockWriter flush_block 从 put_cf 改为 merge_cf
- **H1.1 AsyncBlockWriter**: 后台定时 flush 线程，Group Commit 机制
- **新增测试**: Simple8b(9), Bloom(6), Delta RLE(2), AsyncBlockWriter(1)
- **性能提升**: 单条写入 2.9x, MergeBlock 3.5x, 时间戳压缩 625x
