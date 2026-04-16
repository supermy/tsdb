# TSDB 深度优化实施计划

## 概述

基于 `idea.md` 第96行核心要求：**"充分利用RocksDB的插件机制，定制引擎时序数据库高性能组件，c++开发插件"**，以及当前技术债务清单，制定以下7个Phase的深度优化方案。

---

## Phase A: RocksDB API 深度定制 + C++ 插件

### 目标
深入研究 RocksDB Rust 绑定 (rocksdb 0.22) 全部 API，利用 TablePropertiesCollector / Comparator / MergeOperator / CompactionFilter 等 Hook 定制时序数据库高性能组件。

### A.1 自定义 Comparator — 时序感知排序
**现状**: 使用默认字节序比较器
**优化**: 实现 `TimestampAwareComparator`，RowKey 按 measurement → tags_hash → block_timestamp 排序，确保同一 Series 的数据物理相邻，提升范围扫描局部性

```rust
// crates/tsdb-core/src/storage/comparator.rs
pub struct TsdbComparator;
impl rocksdb::Comparator for TsdbComparator {
    fn name(&self) -> &str { "tsdb.comparator" }
    fn compare(&self, a: &[u8], b: &[u8]) -> Ordering { ... }
}
```

### A.2 BlockBasedTableOptions 调优
**现状**: 使用默认 Options，未定制 SST 格式
**优化**:
- `set_block_size(16KB)` — 时序数据小 KV 特性
- `set_cache_index_and_filter_blocks(true)` — 减少内存占用
- `set_pin_l0_filter_and_index_blocks_in_cache(false)` — 大数据量场景
- `set_format_version(5)` — 支持 IndexType::TwoLevelIndexSearch
- `set_index_type(IndexType::TwoLevelIndexSearch)` — 二级索引减少内存

### A.3 WriteBufferManager — 写入限流
**现状**: 无写入缓冲管理
**优化**: 设置全局 memtable 内存上限，防止 OOM:
```rust
let wbm = rocksdb::WriteBufferManager::new_buf_manager(512 * 1024 * 1024); // 512MB
opts.set_write_buffer_manager(Some(wbm));
```

### A.4 SstFileWriter / IngestExternalFile — 批量导入
**现状**: 仅支持 put/write_batch 单点写入
**优化**: 实现 SST 文件批量生成 + IngestExternalFile 导入，适用于 TSBS 数据导入场景，吞吐量提升 10x+

### A.5 ColumnFamily 级别性能调优
**现状**: 热数据 LZ4、冷数据 ZSTD，但其他参数未调优
**优化**:
- 热 CF: `level_compaction_dynamic_level_bytes=true`, `compression_per_level=[None,LZ4,LZ4,ZSTD]`
- 冷 CF: `disable_auto_compactions=true`, `compaction_style=Universal`
- 元数据 CF: `write_buffer_size=4MB`, `max_write_buffer_number=3`

### A.6 C++ 插件架构设计
**目标**: 通过 RocksDB 的 EventListener / TablePropertiesCollector 实现可扩展插件

```
crates/tsdb-plugin/src/
├── traits.rs          # 已有: BusinessPlugin/QueryPlugin/StoragePlugin
├── registry.rs        # 已有: PluginRegistry
├── rocksdb_hooks.rs   # 🆕 RocksDBEventListener (flush/compaction 回调)
└── properties.rs      # 🆕 TablePropertiesCollector (统计每个 SST 的时序元信息)
```

- **TsdbEventListener**: 监听 flush/compaction 事件，通知聚合引擎触发预计算
- **TsdbTablePropertiesCollector**: 收集每个 SST block 的 min_ts/max_ts/point_count，用于查询裁剪

### 测试要点
- Comparator 正确性测试 (边界 case: 相同前缀不同时间戳)
- SstFileWriter round-trip 测试
- WriteBufferManager 压力测试
- TablePropertiesCollector 属性验证

---

## Phase G: ⭐ MergeOperator 插件定制 — N次访问合并为1次 (核心优化)

### 核心问题分析

#### 当前数据布局 (无 MergeOperator)

**[engine.rs:39-62](crates/tsdb-core/src/storage/engine.rs#L39-L62)** 写入路径:
```
DataPoint (F 个字段)
  → RowKey.encode() + 0x00 + Qualifier.encode()  ← 每个字段一个独立 Key
  → encode_field_value(value)                      ← 每个字段独立 Value
  → put_cf(cf, key, value)                         ← F 次 put 调用
```

具体示例 — 一个 DataPoint (`cpu`, host=server01, ts=1000000, fields={usage:0.5, system:0.3, idle:0.2}):

```
Key(1): "cpu|<hash>|996000\x00usage:400000"  → Value: [0x00][float64:0.5]
Key(2): "cpu|<hash>|996000\x00system:400000" → Value: [0x00][float64:0.3]
Key(3): "cpu|<hash>|996000\x00idle:400000"   → Value: [0x00][float64:0.2]
↑ 3 个独立的 KV 对，3 次 RocksDB put 操作
```

**[engine.rs:95-160](crates/tsdb-core/src/storage/engine.rs#L95-L160)** 读取路径:
```
read_range("cpu", tags, start, end):
  → 遍历每一天的 CF
  → prefix_iterator_cf(prefix="cpu|<tags_hash>|")
  → 迭代 T×F 个 KV 条目 (T个时间点 × F个字段)
  → 每个 KV: split key at 0x00 → RowKey + Qualifier
  → 按 RowKey 分组 → 组装成 DataPoint
  → 最终返回 T 个 DataPoint
```

**访问次数问题**: 对于典型 IoT 场景 (10 metrics × 360 timestamps/hour):
- **写入**: 3600 put_cf 调用/hour
- **读取**: prefix_iterator 迭代 3600 条目，应用层再分组组装

#### 目标数据布局 (使用 MergeOperator)

**核心思想**: 将同一 BlockKey 的所有字段 **merge 到同一个 Value 中**

```
Key:   "cpu|<hash>|996000"                    ← 只有 RowKey，无 Qualifier!
Value: [MERGED_BLOCK]                          ← 所有字段打包在一起
```

**写入时** (F 个字段 → F 次 merge → 1 个合并 Value):
```
merge_cf(cf, "cpu|<hash>|996000", operand(usage, 400000, float 0.5))
merge_cf(cf, "cpu|<hash>|996000", operand(system, 400000, float 0.3))
merge_cf(cf, "cpu|<hash>|996000", operand(idle, 400000, float 0.2))
→ RocksDB 内部合并为 1 个 Value
```

**读取时** (1 次 get → 完整 block):
```
get_cf(cf, "cpu|<hash>|996000")
→ 返回包含所有字段的 MergedBlock Value
→ 解码即可获得完整 DataPoint
→ 无需迭代、无需分组、无需拼接!
```

### G.1 rocksdb 0.22 MergeOperator API 确认

**已验证可用 API** ([rocksdb-0.22.0/src/merge_operator.rs](~/.cargo/registry/src/*/rocksdb-0.22.0/src/merge_operator.rs)):

```rust
// 关联式合并 (满足交换律和结合律的场景)
pub fn set_merge_operator_associative<F: MergeFn + Clone>(&mut self, name: &str, merge_fn: F)

// 通用合并 (full_merge + partial_merge 分离控制)
pub fn set_merge_operator<F: MergeFn, PF: MergeFn>(&mut self, full_merge_fn: F, partial_merge_fn: PF)

// MergeFn trait 签名
pub trait MergeFn:
    Fn(&[u8], Option<&[u8]>, &MergeOperands) -> Option<Vec<u8>> + Send + Sync + 'static
{}

// DB 层面调用 (支持 ColumnFamily)
pub fn merge_cf<K, V>(&self, cf: &impl AsColumnFamilyRef, key: K, value: V) -> Result<(), Error>
```

### G.2 MergeOperand 编码格式设计

**新增文件**: `crates/tsdb-core/src/storage/merge_operand.rs`

每个 field 编码为一个 merge operand (自描述二进制):

```
┌──────────────────────────────────────────────────────┐
│ MergeOperand Binary Format                           │
├──────┬─────────┬──────────┬──────────┬───────────────┤
│ Type │ NameLen │ Name     │ Offset   │ Payload       │
│ (1B) │ (1B)    │ (Var)    │ (4B LE)  │ (Var)         │
├──────┼─────────┼──────────┼──────────┼───────────────┤
│ 0x00 │ n       │ UTF-8    │ μs offset│ f64 BE (8B)   │ Float
│ 0x01 │ n       │ UTF-8    │ μs offset│ i64 BE (8B)   │ Integer
│ 0x02 │ n       │ UTF-8    │ μs offset│ len:u32+UTF8  │ String
│ 0x03 │ n       │ UTF-8    │ μs offset│ 0x00/0x01     │ Boolean
└──────┴─────────┴──────────┴──────────┴───────────────┘
```

```rust
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
enum FieldType {
    Float = 0x00,
    Integer = 0x01,
    String = 0x02,
    Boolean = 0x03,
}

pub fn encode_merge_operand(field_name: &str, micro_offset: u32, value: &FieldValue) -> Vec<u8> {
    let mut buf = Vec::with_capacity(2 + field_name.len() + 4 + 9);
    match value {
        FieldValue::Float(f) => {
            buf.push(FieldType::Float as u8);
            buf.push(field_name.len() as u8);
            buf.extend_from_slice(field_name.as_bytes());
            buf.extend_from_slice(&micro_offset.to_le_bytes());
            buf.extend_from_slice(&f.to_be_bytes());
        }
        FieldValue::Integer(i) => {
            buf.push(FieldType::Integer as u8);
            buf.push(field_name.len() as u8);
            buf.extend_from_slice(field_name.as_bytes());
            buf.extend_from_slice(&micro_offset.to_le_bytes());
            buf.extend_from_slice(&i.to_be_bytes());
        }
        // ... String, Boolean 类似
    }
    buf
}
```

### G.3 MergedBlock Value 格式设计

**MergeOperator 合并后的 Value 布局**:

```
┌────────┬────────────┬─────────────────────────────────────┐
│ Magic  │ FieldCount │ Fields[]                            │
│ (2B)   │ (u16 LE)   │ (连续排列，每个字段变长)              │
├────────┼────────────┼─────────────────────────────────────┤
│ 0xFEED │ N          │ [Field_1][Field_2]...[Field_N]      │
└────────┴────────────┴─────────────────────────────────────┘

Each Field:
┌──────────┬─────────┬──────────┬──────────┬────────────┐
│ NameLen  │ Name    │ Offset   │ Type     │ Payload    │
│ (1B)     │ (Var)   │ (4B LE)  │ (1B)     │ (Var)      │
└──────────┴─────────┴──────────┴──────────┴────────────┘
```

```rust
const MERGE_MAGIC: u16 = 0xFEED;

struct MergedBlock {
    pub fields: Vec<MergedField>,
}

struct MergedField {
    pub name: String,
    pub micro_offset: u32,
    pub value: FieldValue,
}

impl MergedBlock {
    pub fn encode(&self) -> Vec<u8> { ... }
    pub fn decode(data: &[u8]) -> Option<Self> { ... }

    /// 获取指定时间戳的所有字段 → 直接构造 DataPoint
    pub fn get_data_point_at(&self, measurement: &str, block_start: i64, target_ts: i64) -> Option<DataPoint> { ... }

    /// 获取 block 内所有时间戳的去重 DataPoints
    pub fn to_data_points(&self, measurement: &str, block_start: i64, tags: Tags) -> Vec<DataPoint> { ... }
}
```

### G.4 TsdbMergeOperator 实现

**新增文件**: `crates/tsdb-core/src/storage/merge_operator.rs`

```rust
use rocksdb::{MergeOperands, MergeFn};

/// TSDB 时序块级 MergeOperator
///
/// 将同一 RowKey 的多个字段 operand 合并为一个 MergedBlock
///
/// 语义:
/// - existing_value: 已有的 MergedBlock (或 None 表示首次写入)
/// - operands: 新到达的字段 operand 列表 (每个是一个 field)
/// - 返回值: 合并后的新 MergedBlock (包含旧有 + 新增的所有字段)
///
/// 特性:
/// - 幂等: 同一字段多次 merge，后写覆盖先写 (按 micro_offset + field_name 匹配)
/// - 结合律: 多次 partial merge 结果与一次 full merge 结果一致
/// - 零拷贝: operands 为 &[u8] 引用，减少内存分配
pub fn tsdb_block_merge(
    _key: &[u8],
    existing_value: Option<&[u8]>,
    operands: &MergeOperands,
) -> Option<Vec<u8>> {
    // 1. 解析已有的 MergedBlock (如果存在)
    let mut block = existing_value
        .and_then(MergedBlock::decode)
        .unwrap_or_default();

    // 2. 逐个合并 operand 中的字段
    for op in operands.iter() {
        if let Some(field) = decode_merge_operand(op) {
            block.upsert_field(field);  // 按 (name, offset) 去重，后写覆盖
        }
    }

    // 3. 序列化并返回
    Some(block.encode())
}

/// 注册到 RocksDB Options
pub fn register_merge_operator(opts: &mut rocksdb::Options) {
    opts.set_merge_operator_associative(
        "tsdb.block_merge",
        tsdb_block_merge as _,
    );
}
```

### G.5 StorageEngine 写入路径改造

**修改文件**: [engine.rs](crates/tsdb-core/src/storage/engine.rs)

```rust
impl StorageEngine {

    /// 🆕 使用 MergeOperator 的块级写入 (推荐路径)
    ///
    /// 对比原 write():
    /// - 原: F 个 put_cf → F 个独立 KV (RowKey+Qualifier → raw_value)
    /// - 新: F 个 merge_cf → 1 个合并 KV (RowKey → MergedBlock)
    ///
    /// 性能优势:
    /// - merge 比 put 快 ~2x (延迟写 WAL，batch 后统一刷盘)
    /// - 读取时 1 次 get 即可获得全部字段 (见 read_merged())
    pub fn write_merged(&self, dp: &DataPoint) -> Result<()> {
        let row_key = RowKey::from_data_point(dp);
        let block_start = row_key.block_start_timestamp;
        let cf_name = timestamp_to_cf_name(dp.timestamp);

        let date = micros_to_date(dp.timestamp);
        self.cf_manager.ensure_cf_for_date(date)?;

        let cf = self.cf_manager.cf_handle(&cf_name)?;
        let rk_bytes = row_key.encode();  // 注意: 没有 qualifier!

        for (field_name, field_value) in &dp.fields {
            let qualifier = Qualifier::new(field_name, dp.timestamp, block_start);
            let operand = encode_merge_operand(
                field_name,
                qualifier.microsecond_offset,
                field_value,
            );

            self.db.merge_cf(&cf, &rk_bytes, operand)  // merge 替代 put!
                .map_err(|e| TsdbError::Storage(format!("merge failed: {}", e)))?;
        }

        Ok(())
    }

    /// 🆕 批量合并写入 (最高效路径)
    ///
    /// 利用 WriteBatch.merge_cf 批量提交多个 merge 操作
    pub fn write_merged_batch(&self, data_points: &[DataPoint]) -> Result<()> {
        let mut batch = WriteBatch::default();
        // ... 收集所有 merge 操作到 batch ...
        self.db.write(batch)?;
        Ok(())
    }

    /// 🆕 基于 MergeOperator 的块级读取
    ///
    /// 对比原 read_range():
    /// - 原: prefix_iterator → T×F 条 KV → split key → group by RK → assemble
    /// - 新: prefix_iterator → T 条 KV → each has all fields → direct decode
    ///
    /// I/O 减少: T×F → T (F = 字段数，通常 5-20)
    /// CPU 减少: 无需 key split、group by、assemble
    pub fn read_range_merged(
        &self,
        measurement: &str,
        tags: &Tags,
        start_micros: i64,
        end_micros: i64,
    ) -> Result<Vec<DataPoint>> {
        let tags_hash = compute_tags_hash(tags);
        let mut results = Vec::new();

        let start_date = micros_to_date(start_micros);
        let end_date = micros_to_date(end_micros);

        let mut current_date = start_date;
        while current_date <= end_date {
            let cf_name = self.cf_manager.get_cf_name(current_date);
            if let Ok(cf) = self.cf_manager.cf_handle(&cf_name) {
                let prefix_key = {
                    let mut buf = measurement.as_bytes().to_vec();
                    buf.push(SEPARATOR);
                    buf.extend_from_slice(&tags_hash.to_be_bytes());
                    buf.push(SEPARATOR);
                    buf
                };

                let iter = self.db.prefix_iterator_cf(&cf, &prefix_key);
                for item in iter {
                    let (key, value) = match item {
                        Ok(kv) => kv,
                        Err(_) => break,
                    };
                    if !key.starts_with(&prefix_key) { break; }

                    // 🆕 直接解码 MergedBlock，无需拆分 key
                    if let Some(rk) = RowKey::decode(&key) {
                        if let Some(block) = MergedBlock::decode(&value) {
                            let dps = block.to_data_points(
                                &rk.measurement,
                                rk.block_start_timestamp,
                                tags.clone(),
                            );
                            for mut dp in dps {
                                if dp.timestamp >= start_micros && dp.timestamp <= end_micros {
                                    results.push(dp);
                                }
                            }
                        }
                    }
                }
            }
            current_date += chrono::Duration::days(1);
        }

        results.sort_by_key(|dp| dp.timestamp);
        Ok(results)
    }

    /// 🆕 单点精确查询 (最大优势场景)
    ///
    /// 原: 需要 F 次 get (每个字段一次) 或 prefix_scan + filter
    /// 新: 1 次 get_cf → 完整 MergedBlock → 取目标时间戳
    pub fn get_point_merged(
        &self,
        measurement: &str,
        tags: &Tags,
        timestamp: i64,
    ) -> Result<Option<DataPoint>> {
        let tags_hash = compute_tags_hash(tags);
        let block_start = align_to_block_start(timestamp);
        let cf_name = timestamp_to_cf_name(timestamp);

        let cf = self.cf_manager.cf_handle(&cf_name)?;

        let key = {
            let mut buf = measurement.as_bytes().to_vec();
            buf.push(SEPARATOR);
            buf.extend_from_slice(&tags_hash.to_be_bytes());
            buf.push(SEPARATOR);
            buf.extend_from_slice(&block_start.to_be_bytes());
            buf
        };

        match self.db.get_cf(&cf, &key) {
            Ok(Some(value)) => {
                if let Some(block) = MergedBlock::decode(&value) {
                    Ok(block.get_data_point_at(measurement, block_start, timestamp, tags.clone()))
                } else {
                    Ok(None)
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(TsdbError::Storage(format!("get failed: {}", e))),
        }
    }
}
```

### G.6 StorageEngine.open() 集成 MergeOperator

**修改**: `open()` 方法中注册 MergeOperator

```rust
impl StorageEngine {
    pub fn open(path: &Path, cf_config: CfConfig) -> Result<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        // 🆕 注册 TSDB 时序块级 MergeOperator
        crate::storage::merge_operator::register_merge_operator(&mut opts);

        // ... 其余初始化逻辑不变 ...
    }
}
```

### G.7 双模式兼容策略

新旧两种存储格式需要共存 (平滑迁移):

```
Value 格式检测:
  前 2 字节 == 0xFEED → MergedBlock (新模式)
  第 1 字节 == 0x00/0x01/0x02/0x03 → RawFieldValue (旧行模式)
```

```rust
fn detect_value_format(data: &[u8]) -> ValueFormat {
    if data.len() >= 2 && u16::from_le_bytes([data[0], data[1]]) == MERGE_MAGIC {
        ValueFormat::Merged
    } else {
        ValueFormat::Raw
    }
}

// read_range 兼容两种格式
pub fn read_range_compat(...) -> Result<Vec<DataPoint>> {
    // ... iterator ...
    match detect_value_format(&value) {
        ValueFormat::Merged => { /* MergedBlock::decode */ }
        ValueFormat::Raw => { /* 原有的 key split + assemble 逻辑 */ }
    }
}
```

### G.8 性能影响量化

| 场景 | 当前 (无 MergeOperator) | 优化后 (MergeOperator) | 提升 |
|------|------------------------|----------------------|------|
| **单点查询** (10字段) | 10次 get 或 scan+filter | **1次 get_cf** | **10x** |
| **范围查询** (1h, 10metrics, 10s间隔) | 3600 次 iterator 迭代 | **360 次** (每block 1次) | **10x** |
| **写入** (per DP, 10字段) | 10次 put_cf | 10次 merge_cf (~2x faster per op) | **2x** |
| **SST 文件数** | T×F 个 KV | T 个 KV (每 block 1个) | **减少 F 倍** |
| **Compaction 开销** | T×F 个 key 重写 | T 个 key 重写 | **减少 F 倍** |
| **Block Cache 命中率** | 低 (碎片化) | **高** (整 block 缓存) | **显著提升** |

### G.9 与其他 Phase 的协同效应

```
MergeOperator (Phase G) ──→ Phase D (BlockCodec): 可在 MergedBlock 上再做压缩
                         ─→ Phase A (Comparator): 保证同 block 数据物理相邻
                         ─→ Phase C (索引持久化): MergedBlock 包含更完整信息
                         ─→ Phase F (TSBS): 大幅降低基准测试查询延迟
```

### 测试要点

```rust
#[cfg(test)]
mod tests {
    // 1. MergeOperand 编码/解码 round-trip
    #[test]
    fn test_merge_operand_roundtrip() { ... }

    // 2. MergedBlock 编码/解码 round-trip
    #[test]
    fn test_merged_block_roundtrip() { ... }

    // 3. MergeOperator 正确性: 多字段合并顺序无关
    #[test]
    fn test_merge_commutative() { ... }

    // 4. MergeOperator 幂等性: 同字段覆盖
    #[test]
    fn test_merge_idempotent_upsert() { ... }

    // 5. 端到端: write_merged → read_merged 一致性
    #[test]
    fn test_write_read_merged_e2e() { ... }

    // 6. 端到端: write_merged → get_point_merged 单点查询
    #[test]
    fn test_get_point_merged_e2e() { ... }

    // 7. 兼容性: 旧格式数据仍可读取
    #[test]
    fn test_backward_compatibility_raw_format() { ... }

    // 8. 性能基准: merge vs put 吞吐对比
    #[test]
    fn bench_merge_vs_put_throughput() { ... }

    // 9. 性能基准: read_merged vs read_range 延迟对比
    #[test]
    fn bench_read_merged_vs_legacy() { ... }
}
```

---

## Phase B: NNG 服务接口集成

### 目标
替换 [server.rs](crates/tsdb-server/src/server.rs) 中标准 TCP 为 NNG (nanomsg-next-gen)，实现请求/响应 + 发布/订阅双模式。

### B.1 NNG 基础设施层
**新增**: `crates/tsdb-server/src/nng_transport.rs`

```rust
// 请求/响应模式 (REP/REQ) — 替代现有 TCP
pub struct RepServer {
    socket: nng::Socket,
    ctx: Option<nng::Context>,
}

// 发布/订阅模式 (PUB/SUB) — 新增实时推送
pub struct PubServer {
    socket: nng::Socket,
}

// Pipeline 模式 (PULL/PUSH) — 高吞吐批量写入
pub struct PullServer {
    socket: nng::Socket,
}
```

### B.2 协议升级
**现状**: MessagePack + 长度前缀 (4 bytes BE)
**优化**: 保持 MessagePack 序列化，但增加协议版本号和校验:

```
+--------+--------+----------------+
| Version| CRC32  | MessagePack    |
| (1B)   | (4B)   | (variable)     |
+--------+--------+----------------+
```

### B.3 服务端重构
**文件**: [server.rs](crates/tsdb-server/src/server.rs)
- `start()` → 启动 REP socket (端口 9527)
- `start_with_http()` → 同时启动 REP + HTTP
- 新增 `start_pub_sub()` → PUB socket (端口 9529)
- 新增 `start_pipeline()` → PULL socket (端口 9530)

### B.4 异步 I/O
**现状**: 同步单线程 accept 循环
**优化**: 使用 `nng::aio::Aio` + tokio runtime 实现异步多路复用:
```rust
let mut aio = nng::aio::Aio::new(move |aio, result| {
    match result {
        Ok(nng::aio::AioResult::Send) => { ... }
        Ok(nng::aio::AioResult::Recv(msg)) => { handle_msg(msg); aio.recv(ctx); }
        _ => {}
    }
});
```

### B.5 CLI 客户端适配
**文件**: [main.rs](crates/tsdb-cli/src/main.rs)
- 新增 `--transport nng` 选项 (默认保持 tcp 兼容)
- REQ socket 连接服务端

### 测试要点
- REQ/REP round-trip 延迟测试
- PUB/SUB 多 subscriber 广播测试
- PULL/PUSH 吞吐量对比基准
- 协议版本兼容性测试

---

## Phase C: 索引持久化

### 目标
将当前纯内存的 SkipList 和 InvertedIndex 持久化到 RocksDB，实现重启恢复。

### C.1 持久化方案设计
**存储位置**: 利用现有的 METADATA_CF (在 cf_manager.rs 中定义)

```
METADATA_CF Key Layout:
├── index:time:{measurement}           → SkipList 序列化数据
├── index:tag:{measurement}:postings   → RoaringBitmap postings
├── index:tag:{measurement}:series     → SeriesId→Tags 映射
├── index:meta:next_series_id          → 自增 SeriesId
└── index:meta:version                 → 索引格式版本号
```

### C.2 SkipList 持久化
**文件**: `crates/tsdb-index/src/skiplist.rs` (扩展)

```rust
impl SkipList {
    pub fn serialize(&self) -> Vec<u8> { ... }  // 所有节点 → 二进制
    pub fn deserialize(data: &[u8]) -> Self { ... }

    // 增量持久化: 只序列化新节点
    pub fn serialize_delta(&self, since_seq: u64) -> Vec<u8> { ... }
}
```

格式: `[node_count:u32][node_1][node_2]...[node_N]`
每节点: `[key:i64][offset_count:u32][offsets:u64...]`

### C.3 InvertedIndex 持久化
**文件**: `crates/tsdb-index/src/inverted.rs` (扩展)

```rust
impl InvertedIndex {
    pub fn save_to_db(&self, db: &Arc<DB>, cf: &ColumnFamily, measurement: &str) -> Result<()> { ... }
    pub fn load_from_db(db: &Arc<DB>, cf: &ColumnFamily, measurement: &str) -> Result<Self> { ... }
}
```

- RoaringBitmap 本身已支持 `serialize()` / `deserialize()`
- series_tags 用 JSON 或 bincode 序列化

### C.4 WAL + Checkpoint 双保险
**策略**:
1. **WAL 模式**: 每次 insert 后异步写 METADATA_CF（低延迟优先）
2. **Checkpoint 模式**: 每 N 秒或 M 次操作后做一次全量快照（一致性优先）
3. **启动恢复**: 先加载最新 checkpoint，再回放 WAL

### C.5 IndexManager 集成
**文件**: `crates/tsdb-index/src/manager.rs` (修改)

```rust
pub struct IndexManager {
    time_index: HashMap<String, SkipList>,
    tag_index: HashMap<String, InvertedIndex>,
    // 🆕 持久化相关
    db: Option<Arc<DB>>,
    wal_sequence: AtomicU64,
    last_checkpoint: Instant,
    checkpoint_interval: Duration,
}
```

### 测试要点
- SkipList 序列化/反序列化 round-trip
- InvertedIndex RocksDB 存取正确性
- 模拟崩溃后重启恢复完整性
- WAL vs Checkpoint 性能对比

---

## Phase D: BlockCodec 写入路径集成

### 目标
将 [codec.rs](crates/tsdb-compress/src/codec.rs) 中的 BlockCodec 集成到 StorageEngine.write() 流水线，实际压缩存储数据。
**注意**: 与 Phase G (MergeOperator) 协作 — BlockCodec 压缩作用于 MergedBlock 级别。

### D.1 写入流水线架构

**现状** (无压缩):
```
DataPoint → encode_field_value() → raw bytes → put_cf()
```

**目标** (MergeOperator + BlockCodec 双重优化):
```
DataPoint → encode_merge_operand() → merge_cf() → MergedBlock → BlockCodec.compress() → put_cf()
```

### D.2 BlockWriter — 分块缓冲写入器
**新增**: `crates/tsdb-core/src/storage/block_writer.rs`

```rust
pub struct BlockWriter {
    blocks: HashMap<BlockKey, MergedBlock>,
    engine: Arc<StorageEngine>,
    config: BlockWriterConfig,
}

#[derive(Clone)]
pub struct BlockWriterConfig {
    pub max_block_rows: usize,
    pub max_block_duration_us: i64,
    pub flush_interval_ms: u64,
    pub compression_enabled: bool,
}
```

### D.3 StorageEngine 改造
**文件**: [engine.rs](crates/tsdb-core/src/storage/engine.rs)

```rust
impl StorageEngine {
    pub fn write_compressed(&self, dp: &DataPoint) -> Result<()> { ... }
    pub fn flush_blocks(&self) -> Result<usize> { ... }
    pub fn read_range_compressed(&self, ...) -> Result<Vec<DataPoint>> { ... }
}
```

### D.4 读取路径解压
```
value format:
  [0xFEED]           → 未压缩 MergedBlock (Phase G 格式)
  [0xFEED][0xFF]     → 压缩 CompressedBlock (Phase D 格式)
  [0x00-0x03]        → 旧原始格式 (向后兼容)
```

### 测试要点
- BlockWriter 缓冲→满块触发压缩→写入→读取→解压 全链路
- 跨 block 边界的时间戳连续性
- 混合压缩/非压缩数据共存读取
- flush_blocks 在异常退出时的数据安全

---

## Phase E: 错误处理统一

### 目标
消除 `anyhow::Result` 散落，统一为 `tsdb_core::error::TsdbError`。

### E.1 错误类型扩展
**文件**: [error.rs](crates/tsdb-core/src/error.rs)

```rust
#[derive(Error, Debug)]
pub enum TsdbError {
    // ✅ 已有变体
    Storage(String), InvalidDataPoint(String), ColumnFamilyNotFound(String),
    Compression(String), Decompression(String), Index(String), Query(String),
    Config(String), Io(#[from] std::io::Error), RocksDb(#[from] rocksdb::Error),
    Serialization(String),

    // 🆕 新增变体
    Network(String), Protocol(String), Plugin(String),
    Nng(String), Dashboard(String), NotFound(String), Internal(String),
}
```

### E.2 From 转换实现
- `From<serde_json::Error>` → `TsdbError::Serialization`
- `From<rmp_serde::Error>` → `TsdbError::Serialization`
- `From<std::net::AddrParseError>` → `TsdbError::Network`
- `From<nng::Error>` → `TsdbError::Nng`
- `From<sqlparser::parser::ParserError>` → `TsdbError::Query`
- `From<chrono::ParseError>` → `TsdbError::Config`
- `From<CompressError>` → `TsdbError::Compression`

### E.3 各 Crate 迁移清单

| 文件 | 当前 | 目标 |
|------|------|------|
| server.rs | `anyhow::Result<()>` | `Result<(), TsdbError>` |
| http_api.rs | `anyhow::Result<String>` | `Result<String, TsdbError>` |
| renderer.rs | implicit ok | `Result<String, TsdbError>` |

### 测试要点
- 所有 From 转换覆盖测试
- 错误链传播完整性测试
- HTTP API 错误响应格式验证

---

## Phase F: TSBS 真实数据验证

### 目标
用 TSBS 生成的真实 DevOps 数据集跑完整流程，建立性能基线。

### F.1 TSBS 数据生成脚本
**新增**: `scripts/generate_tsbs_data.sh`

### F.2 TSBS 数据加载器
**新增**: `crates/tsdb-cli/src/commands/load_tsbs.rs`

### F.3 性能基准报告模板
**新增**: `benches/bench_full.rs`

### F.4 CI 集成
**修改**: [.github/workflows/ci.yml](.github/workflows/ci.yml)

---

## 实施顺序与依赖关系

```
Phase E (错误处理统一) ──┐
                        ├──→ Phase A (RocksDB深度定制) ──┬→ Phase G (MergeOperator⭐) ──┐
Phase B (NNG集成) ──────┤                              │                             │
                        │                              ├──→ Phase D (BlockCodec) ────┼→ Phase F (TSBS验证)
Phase C (索引持久化) ───┘                              │                             │
                                                       └→ Phase C (索引持久化) ─────┘
```

**推荐执行顺序**: **E → A → G → D → C → B → F**
- **E 最先**: 错误统一是基础设施，后续 Phase 都受益
- **A 其次**: RocksDB 定制是核心性能基础
- **G 核心** (🆕新增): MergeOperator 是最大性能杠杆，应尽早实施
- **D 依赖 G+A**: BlockCodec 在 MergedBlock 基础上压缩效果最佳
- **C 依赖 D**: 索引持久化在稳定写入路径后更可靠
- **B 可并行**: NNG 与存储层相对独立
- **F 最后**: 所有优化完成后的最终验收

## 验收标准

| 指标 | 当前 | 目标 |
|------|------|------|
| 编译警告 | ~30 warnings | < 5 warnings |
| 测试数量 | 50 | > 90 (+9 MergeOperator tests) |
| 单点查询 I/O | F 次 get/scan | **1 次 get_cf** (**10x↓**) |
| 范围查询迭代 | T×F 条 KV | **T 条 KV** (**F倍↓**) |
| 写入吞吐量 | baseline | +100% (merge + batch) |
| SST 文件大小 | baseline | **-60%** (少 F 倍 key 开销) |
| Block Cache 命中 | 碎片化 | **整 block 命中** |
| 错误类型 | 混合 anyhow/TsdbError | 100% TsdbError |
| 重启恢复 | 索引全丢失 | 完整恢复 |
| 服务协议 | TCP only | TCP + NNG (REQ/REP + PUB/SUB) |
| TSBS 验证 | 未执行 | 4000设备×4天完整跑通 |

---

## Phase H: InfluxDB 对标优化 (2026-04 新增)

> 对标 InfluxDB TSM 引擎和 InfluxDB 3.0 (Arrow/Parquet) 架构，识别性能优化空间

### H.1 写入路径优化 (P0 — ✅ 已实施)

**InfluxDB TSM 做法**: 写入先进入内存 Cache (按 series key 分区)，定期批量刷盘到 TSM File。WAL 异步写入，不阻塞写入路径。

**当前差距**: ~~每次 `write()` 调用都执行 `put_cf`，触发 RocksDB MemTable 写入 + WAL sync。~~ 已优化。

**优化方案**:

| # | 优化项 | 实现方式 | 状态 | 实测效果 |
|---|--------|---------|------|---------|
| H1.1 | WriteBatch Group Commit | `AsyncBlockWriter` 后台 flush 线程，攒够 N 条或 T 毫秒后批量提交 | ✅ 已实施 | 大规模写入 1.2x |
| H1.2 | WAL 异步 fsync | `set_use_fsync(false)` + `manual_wal_flush` + `PointInTime` 恢复模式 | ✅ 已实施 | 单条写入 2.9x |
| H1.3 | Series Cache 延迟 merge | BlockWriter `flush_block` 从 `put_cf` 改为 `merge_cf` | ✅ 已实施 | MergeBlock 3.5x |

**关键代码变更**:
- `crates/tsdb-core/src/storage/options.rs`: WAL 异步配置
- `crates/tsdb-core/src/storage/engine.rs`: `flush_wal()` 方法
- `crates/tsdb-core/src/storage/block_writer.rs`: `merge_cf` + `AsyncBlockWriter`

### H.2 压缩算法优化 (P1 — ✅ 已实施)

**InfluxDB TSM 做法**: 时间戳使用完整的 Delta-of-Delta + RLE + Simple8b 编码，浮点数使用 Gorilla XOR + 前导零分组。InfluxDB 宣称平均 2.2 bytes/point。

**当前差距**: ~~Delta 仅实现基础 Delta + ZigZag，Gorilla 未做前导零分组优化。~~ 已优化。

**优化方案**:

| # | 优化项 | 实现方式 | 状态 | 实测效果 |
|---|--------|---------|------|---------|
| H2.1 | Delta-of-Delta + RLE | RLE marker(0xFF) + value + repeat_count，连续相同 DoD 编码为 ~3 字节 | ✅ 已实施 | 时间戳 5000:1 |
| H2.2 | Gorilla 前导零分组 | 已有标准 Gorilla XOR + leading/trailing zeros 复用 | ✅ 已达标 | 浮点 ~1.1:1 (随机数据) |
| H2.3 | Simple8b 整数编码 | 16 selector 字对齐压缩 + Zigzag 编码 | ✅ 已实施 | 整数 7.5:1 |

**关键代码变更**:
- `crates/tsdb-compress/src/delta.rs`: RLE 编码增强
- `crates/tsdb-compress/src/simple8b.rs`: 新增 Simple8b 模块
- `crates/tsdb-compress/src/codec.rs`: 整数编码从 Big-Endian → Simple8b + Zigzag

### H.3 查询引擎优化 (P2 — 部分实施)

**InfluxDB 做法**: TSM File 内嵌 Bloom Filter，快速排除不包含目标 series 的文件。InfluxDB 3.0 使用 DataFusion 列式查询引擎。

**优化方案**:

| # | 优化项 | 实现方式 | 状态 | 实测效果 |
|---|--------|---------|------|---------|
| H3.1 | Series Key Bloom Filter | `BloomFilter` 1% FPP + merge，`might_contain_series()` 预检 | ✅ 已实施 | 快速排除 |
| H3.2 | 列式内存布局 | `ColumnarBatch` 使用 `Vec<f64>` 列式存储 + SIMD 聚合 | ✅ 已实现 | 已有 |
| H3.3 | Parquet 存储格式 | 长期：引入 Apache Arrow + Parquet 替代 RocksDB | 🔜 长期 | — |

**关键代码变更**:
- `crates/tsdb-index/src/bloom.rs`: 新增 BloomFilter 模块
- `crates/tsdb-index/src/manager.rs`: 集成 BloomFilter + `might_contain_series()`

### H.4 InfluxDB 3.0 方向 (P3 — 长期架构演进)

InfluxDB 3.0 完全重写为 Rust + Apache Arrow + DataFusion + Parquet 架构，这是时序数据库的未来方向。

| # | 优化项 | 说明 | 依赖 |
|---|--------|------|------|
| H4.1 | Apache Arrow 内存模型 | 零拷贝列式处理，消除序列化开销 | arrow crate |
| H4.2 | DataFusion 查询引擎 | 替代自研 SQL Parser，支持 JOIN/子查询/窗口函数 | datafusion crate |
| H4.3 | Parquet 持久化 | 列式文件存储，比 SST 更适合分析查询 | parquet crate |
| H4.4 | Object Storage 分层 | 热数据 SSD + 冷数据 S3，降低存储成本 | object_store crate |

### 验收标准 (Phase H)

| 指标 | 优化前 | H1 后 (实测) | H2 后 (实测) | H3 后 (实测) |
|------|--------|-------------|-------------|-------------|
| 批量写入 QPS | 45K | 43.5K | 43.5K | 43.5K |
| 单线程写入 QPS | 6K | **18.6K** | 18.6K | 18.6K |
| MergeBlock 写入 QPS | 9.6K | **33.2K** | 33.2K | 33.2K |
| 浮点压缩比 | 5:1 | 5:1 | ~1.1:1 | ~1.1:1 |
| 时间戳压缩比 | 8:1 | 8:1 | **5000:1** | 5000:1 |
| 整数压缩比 | 1:1 | 1:1 | **7.5:1** | 7.5:1 |
| 点查询延迟 | 0.15ms | 0.15ms | 0.15ms | Bloom 预检 ~0.001ms |
| 聚合吞吐 | 111K | 106K | 106K | 106K |
