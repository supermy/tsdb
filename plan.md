# TSDB 综合实施计划 — 完成全部剩余功能

> **评审日期**: 2026-04-16
> **当前状态**: 0 编译错误, 0 警告, 77 测试通过
> **计划依据**: idea.md + plan.md + deep-optimize-plan.md 三文档交叉评审

---

## 一、实现现状总览

### ✅ 已完成（26/31 项）

| # | 需求 | 实现位置 | 验证 |
|---|------|---------|------|
| A.1 | TsdbComparator 时序感知排序 | comparator.rs | ✅ 5 tests |
| A.2 | BlockBasedTableOptions 调优 | options.rs | ✅ |
| A.3 | WriteBufferManager 写入限流 | options.rs | ✅ |
| A.4 | SstFileWriter 批量导入 | bulk_import.rs | ✅ 2 tests |
| A.5 | CF 级别性能调优 | options.rs (hot/cold/metadata) | ✅ |
| A.6 | rocksdb_hooks + properties | rocksdb_hooks.rs, properties.rs | ✅ 5 tests |
| G.1 | MergeOperator + MergeOperand | merge_operator.rs, merge_operand.rs | ✅ 8 tests |
| G.2 | MergedBlock 格式 | merge_operand.rs (0xFEED 魔数) | ✅ |
| G.3 | write_merged / write_merged_batch | engine.rs | ✅ |
| G.4 | detect_value_format 双模式兼容 | merge_operand.rs, engine.rs | ✅ |
| B.1 | NNG REP/PUB/PULL 三协议 | nng_transport.rs | ✅ |
| B.2 | 协议 V2 版本 + CRC32 | protocol.rs (Envelope) | ✅ |
| C.1 | IndexManager 序列化/反序列化 | manager.rs | ✅ |
| C.2 | StorageEngine persist/restore index | engine.rs | ✅ |
| D.1 | BlockWriter 缓冲写入 | block_writer.rs | ✅ |
| D.2 | write/read_compressed_block | engine.rs | ✅ |
| D.3 | read_range_compressed | engine.rs | ✅ |
| E.1 | TsdbError 18 个变体 | error.rs | ✅ |
| E.3 | 核心 crate 消除 anyhow | server.rs, http_api.rs | ✅ |
| 1.1 | 多业务 DB 隔离 | multi_db.rs, server.rs | ✅ 6 tests |
| 1.2 | 轻度汇总异步管道 | pipeline.rs, worker.rs | ✅ 2 tests |
| 1.3 | 聚合独立存储 | store.rs | ✅ 3 tests |
| 2.1 | 向量化引擎接入查询 | engine.rs (QueryEngine) | ✅ |
| 2.2 | 前缀 Key 压缩 | key_codec.rs | ✅ 4 tests |
| 3.1/3.2 | ExpoGo 仪表盘 + 多序列图 | App.js | ✅ |
| F.0 | TSBS 数据生成 + CLI 加载 | main.rs load-tsbs | ✅ |

### 🔄 部分完成（3 项）

| # | 需求 | 差距 | 优先级 |
|---|------|------|--------|
| B.3 | NNG 与 TsdbServer 集成 | NngServer 独立存在，未在 TsdbServer.start() 中自动启动 | P1 |
| E.2 | From 转换补全 | 缺少 nng/bincode/rmp_serde/CompressError/QueryError 的自动 From | P2 |

### ❌ 未完成（2 项）

| # | 需求 | 说明 | 优先级 |
|---|------|------|--------|
| B.4 | 异步 I/O | 全同步阻塞，高并发受限 | P1 |
| C.3 | WAL + Checkpoint | 无应用层 WAL，崩溃时索引可能丢失 | P2 |

---

## 二、剩余任务实施计划

### 任务 1: NNG 与 TsdbServer 集成（B.3 补全）

**目标**: TsdbServer 启动时自动启动 NNG 服务，统一管理所有传输层

**改动文件**:
- `tsdb-server/src/server.rs`: 在 `start_with_http()` 中增加 NNG 启动逻辑
- `tsdb-server/src/lib.rs`: 导出 NNG 相关类型

**具体改动**:
```rust
// server.rs - TsdbServer 新增方法
pub fn start_with_nng(&mut self, rep_port: u16, pull_port: u16, pub_port: u16) -> Result<()>
pub fn start_all(&mut self, http_port: u16, nng_rep_port: u16, nng_pull_port: u16, nng_pub_port: u16) -> Result<()>
```

**验收标准**: `TsdbServer::start_all()` 启动后，TCP/HTTP/NNG 三种协议同时可用

---

### 任务 2: 异步 I/O 改造（B.4）

**目标**: 将服务端从同步阻塞改为 tokio 异步，支持高并发连接

**改动文件**:
- `tsdb-server/src/server.rs`: TcpListener → tokio::net::TcpListener + async handle
- `tsdb-server/src/http_api.rs`: 同步 handler → async handler (hyper/axum)
- `tsdb-server/src/nng_transport.rs`: 同步循环 → tokio::task::spawn_blocking
- `tsdb-server/Cargo.toml`: 添加 tokio 全功能依赖

**架构**:
```
TsdbServer (async main)
├── tokio::spawn → TCP handler (async read/write)
├── tokio::spawn → HTTP handler (axum routes)
├── tokio::task::spawn_blocking → NNG REP loop
├── tokio::task::spawn_blocking → NNG PULL loop
└── tokio::task::spawn_blocking → NNG PUB publisher
```

**验收标准**: 服务端可同时处理 1000+ 并发连接，无阻塞

---

### 任务 3: From 转换补全（E.2）

**目标**: 为所有外部错误类型实现 `From<OuterError> -> TsdbError`

**改动文件**:
- `tsdb-core/src/error.rs`: 新增 5 个 From impl

**具体改动**:
```rust
impl From<bincode::Error> for TsdbError { ... }        // → Serialization
impl From<rmp_serde::decode::Error> for TsdbError { ... } // → Serialization
impl From<rmp_serde::encode::Error> for TsdbError { ... } // → Serialization
// nng::Error 和 CompressError 需在各自 crate 中定义转换
```

**验收标准**: 所有 `map_err(|e| TsdbError::Xxx(...))` 可替换为 `?` 操作符

---

### 任务 4: WAL + Checkpoint 机制（C.3）

**目标**: 实现应用层 WAL + 定期 Checkpoint，保证索引数据崩溃恢复

**改动文件**:
- 新增 `tsdb-index/src/wal.rs`: IndexWAL 实现
- `tsdb-index/src/manager.rs`: 集成 WAL 写入和恢复逻辑

**WAL 格式**:
```
[WAL Entry]
├── entry_len: u32 LE
├── entry_type: u8 (0=Insert, 1=Delete, 2=Checkpoint)
├── sequence: u64 LE
├── payload: Vec<u8> (序列化的操作数据)
└── crc32: u32 LE
```

**Checkpoint 策略**:
- 每 60 秒或每 10000 次操作做一次全量快照
- 快照写入 METADATA_CF 的 `index:checkpoint:<seq>` key
- 恢复时：加载最新 checkpoint → 回放后续 WAL

**验收标准**: 模拟 kill -9 后重启，索引数据完整恢复

---

## 三、实施优先级

| 顺序 | 任务 | 工作量 | 依赖 |
|------|------|--------|------|
| **1** | B.3 NNG 与 TsdbServer 集成 | 小 | 无 |
| **2** | E.2 From 转换补全 | 小 | 无 |
| **3** | B.4 异步 I/O 改造 | 大 | 依赖 B.3 |
| **4** | C.3 WAL + Checkpoint | 中 | 无 |

**建议**: 任务 1+2 可并行完成（小改动），任务 3 是最大工作量（异步改造），任务 4 独立可并行。

---

## 四、验收标准汇总

| 指标 | 当前 | 目标 |
|------|------|------|
| 编译警告 | 0 | 0 |
| 测试数量 | 77 | > 90 |
| 服务协议 | TCP + HTTP + NNG(独立) | TCP + HTTP + NNG(集成) |
| 并发能力 | 同步阻塞 | 异步高并发 |
| 错误处理 | 部分 map_err | 全部 ? 操作符 |
| 索引恢复 | persist/restore (手动) | WAL + Checkpoint (自动) |
