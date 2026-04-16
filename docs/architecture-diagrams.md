# TSDB 架构可视化文档

> 使用 Mermaid 语法，可在 GitHub / VS Code / IntelliJ 中直接渲染

---

## 1. 系统架构图 (System Architecture)

```mermaid
flowchart TB
    subgraph Client["客户端层"]
        CLI["tsdb-cli 命令行工具"]
        HTTP_CLIENT["HTTP Client REST API"]
        TCP_CLIENT["TCP Client 二进制协议V2"]
    end

    subgraph Server["服务层 tsdb-server"]
        HTTP_API["HTTP API warp框架<br/>/api/v1/write /api/v1/query /api/v1/timeseries"]
        TCP_HANDLER["TCP Handler NNG传输 Envelope V2"]
        PROTOCOL["Protocol V2 Envelope CRC32 Request Response MessagePack"]
        TSDB_SERVER["TsdbServer 请求路由"]
    end

    subgraph Query["查询引擎 tsdb-query"]
        PARSER["SqlParser SQL to AST"]
        PLANNER["QueryPlanner AST to ExecutionPlan"]
        QENGINE["QueryEngine 执行引擎"]
        VEC_ENGINE["VectorizedEngine 列式计算"]
        SIMD_AGG["SimdAggregator 聚合函数"]
        COL_BATCH["ColumnarBatch 列式存储"]
    end

    subgraph Aggregate["聚合管道 tsdb-aggregate"]
        PIPELINE["LightAggregationPipeline 内存缓冲 定时刷盘"]
        AGGREGATOR["Aggregator 分桶累加"]
        WORKER["Worker NNG消费者"]
        AGG_STORE["AggregationStore RocksDB持久化"]
        TS_GEN["TimeseriesGenerator 趋势 对比 多维"]
        STORE_MGR["AggregationStoreManager 多业务管理"]
    end

    subgraph Core["存储引擎 tsdb-core"]
        ENGINE["StorageEngine 读写入口"]
        MULTI_DB["MultiDbManager 多数据库管理"]
        CF_MGR["CfManager 按日期CF管理 HOT COLD分层"]
        BLOCK_WRITER["BlockWriter 块缓冲写入"]
        DIM_TABLE["DimensionTable Tag维度编码"]
        MERGE_OP["MergeOperator MergedBlock合并"]
        ROWKEY["RowKey Qualifier 键编码"]
    end

    subgraph Index["索引层 tsdb-index"]
        IDX_MGR["IndexManager 时间 标签索引"]
        SKIPLIST["SkipList 时间范围索引 Ologn"]
        INV_IDX["InvertedIndex 倒排索引 RoaringBitmap"]
        WAL["IndexWAL 预写日志"]
    end

    subgraph Compress["压缩层 tsdb-compress"]
        CODEC["BlockCodec 块级编解码"]
        GORILLA["GorillaEncoder 浮点XOR压缩"]
        DELTA["DeltaEncoder 时间戳Delta压缩"]
        DICT["DictionaryEncoder 字符串字典编码"]
    end

    subgraph Storage["持久化层"]
        ROCKSDB["RocksDB LSM Tree 引擎"]
        AGG_ROCKSDB["RocksDB 聚合数据实例"]
    end

    subgraph Viz["可视化层"]
        CHART["tsdb-chart TimeSeriesChart SvgRenderer"]
        DASHBOARD["tsdb-dashboard Business Performance DashboardRenderer"]
    end

    CLI --> HTTP_API
    CLI --> TCP_HANDLER
    HTTP_CLIENT --> HTTP_API
    TCP_CLIENT --> TCP_HANDLER

    HTTP_API --> TSDB_SERVER
    TCP_HANDLER --> PROTOCOL
    PROTOCOL --> TSDB_SERVER

    TSDB_SERVER --> QENGINE
    TSDB_SERVER --> ENGINE
    TSDB_SERVER --> PIPELINE
    TSDB_SERVER --> MULTI_DB

    QENGINE --> PARSER
    QENGINE --> PLANNER
    QENGINE --> VEC_ENGINE
    VEC_ENGINE --> SIMD_AGG
    VEC_ENGINE --> COL_BATCH

    PIPELINE --> AGGREGATOR
    PIPELINE --> STORE_MGR
    WORKER --> AGGREGATOR
    AGGREGATOR --> AGG_STORE
    AGG_STORE --> AGG_ROCKSDB
    TS_GEN --> AGG_STORE
    TS_GEN --> CHART

    ENGINE --> CF_MGR
    ENGINE --> BLOCK_WRITER
    ENGINE --> DIM_TABLE
    ENGINE --> IDX_MGR
    ENGINE --> CODEC
    ENGINE --> ROWKEY
    ENGINE --> MERGE_OP
    MULTI_DB --> ENGINE

    IDX_MGR --> SKIPLIST
    IDX_MGR --> INV_IDX
    IDX_MGR --> WAL

    CODEC --> GORILLA
    CODEC --> DELTA
    CODEC --> DICT

    ENGINE --> ROCKSDB

    DASHBOARD --> QENGINE
    DASHBOARD --> CHART
```

---

## 2. Crate 依赖层次图 (Dependency Layers)

```mermaid
graph TD
    subgraph L0["Layer 0 — 基础类型"]
        TYPES["tsdb-types"]
        CONFIG["tsdb-config"]
    end

    subgraph L1["Layer 1 — 核心能力"]
        COMPRESS["tsdb-compress"]
        INDEX["tsdb-index"]
        PLUGIN["tsdb-plugin"]
        CHART["tsdb-chart"]
    end

    subgraph L2["Layer 2 — 存储引擎"]
        CORE["tsdb-core"]
    end

    subgraph L3["Layer 3 — 查询/聚合"]
        QUERY["tsdb-query"]
        AGG["tsdb-aggregate"]
        DASH["tsdb-dashboard"]
    end

    subgraph L4["Layer 4 — 服务"]
        SERVER["tsdb-server"]
    end

    subgraph L5["Layer 5 — 客户端/测试"]
        CLI["tsdb-cli"]
        TESTS["tsdb-integration-tests"]
    end

    COMPRESS --> TYPES
    INDEX --> TYPES
    PLUGIN --> TYPES
    CHART --> TYPES

    CORE --> TYPES
    CORE --> CONFIG
    CORE --> COMPRESS
    CORE --> INDEX

    QUERY --> TYPES
    QUERY --> CORE
    QUERY --> INDEX

    AGG --> TYPES
    AGG --> CONFIG
    AGG --> CORE
    AGG --> CHART

    DASH --> TYPES
    DASH --> CORE
    DASH --> QUERY
    DASH --> CHART

    SERVER --> TYPES
    SERVER --> CORE
    SERVER --> CONFIG
    SERVER --> QUERY
    SERVER --> AGG
    SERVER --> CHART
    SERVER --> DASH

    CLI --> TYPES
    CLI --> CORE
    CLI --> CONFIG
    CLI --> QUERY
    CLI --> SERVER
    CLI --> COMPRESS
    CLI --> INDEX
    CLI --> AGG
    CLI --> CHART
    CLI --> DASH

    TESTS --> CORE
    TESTS --> QUERY
    TESTS --> INDEX
    TESTS --> COMPRESS
    TESTS --> AGG
    TESTS --> SERVER
    TESTS --> TYPES
    TESTS --> CONFIG
    TESTS --> CHART

    style L0 fill:#e8f5e9,stroke:#4caf50
    style L1 fill:#e3f2fd,stroke:#2196f3
    style L2 fill:#fff3e0,stroke:#ff9800
    style L3 fill:#f3e5f5,stroke:#9c27b0
    style L4 fill:#fce4ec,stroke:#e91e63
    style L5 fill:#efebe9,stroke:#795548
```

---

## 3. 核心数据模型类图 (Class Diagram)

```mermaid
classDiagram
    class DataPoint {
        +String measurement
        +Tags tags
        +Fields fields
        +Timestamp timestamp
        +new(measurement, timestamp) DataPoint
        +with_tag(key, value) DataPoint
        +with_field(key, value) DataPoint
        +series_key() String
    }

    class FieldValue {
        <<enum>>
        Float(f64)
        Integer(i64)
        String(String)
        Boolean(bool)
        +as_f64() Option~f64~
        +as_i64() Option~i64~
        +as_str() Option~&str~
        +as_bool() Option~bool~
    }

    class Tags {
        <<type alias>>
        BTreeMap~String, String~
    }

    class Fields {
        <<type alias>>
        BTreeMap~String, FieldValue~
    }

    class RowKey {
        +String measurement
        +u64 tags_hash
        +Timestamp block_start_timestamp
        +from_data_point(dp) RowKey
        +encode() Vec~u8~
        +decode(data) RowKey
    }

    class Qualifier {
        +String field_name
        +u32 microsecond_offset
        +new(field_name, timestamp, block_start) Qualifier
        +encode() Vec~u8~
        +decode(data) Qualifier
    }

    class MergedBlock {
        +Vec~MergedField~ fields
        +encode() Vec~u8~
        +decode(data) MergedBlock
        +upsert_field(field) MergedBlock
        +to_data_points(rk) Vec~DataPoint~
        +get_data_point_at(ts, block_start) Option~DataPoint~
    }

    class MergedField {
        +String name
        +u32 micro_offset
        +FieldValue value
    }

    class StorageEngine {
        -Arc~TsdbDB~ db
        -CfManager cf_manager
        +open(path, cf_config) Result~StorageEngine~
        +write(dp) Result~
        +write_batch(dps) Result~
        +write_merged(dp) Result~
        +read_range(measurement, tags, start, end) Result~Vec~DataPoint~~
        +get_point_merged(measurement, tags, ts) Result~
        +write_compressed_block(rk, block) Result~
        +read_compressed_block(rk) Result~
        +cleanup() Result~
        +persist_index(idx_mgr) Result~
        +restore_index(idx_mgr) Result~
    }

    class MultiDbManager {
        -PathBuf data_dir
        -CfConfig cf_config
        -RwLock~HashMap~ databases
        +new(data_dir, cf_config) MultiDbManager
        +create_database(name) Result~Arc~StorageEngine~~
        +get_database(name) Result~Arc~StorageEngine~~
        +drop_database(name) Result~
        +list_databases() Vec~String~
        +ensure_default() Result~Arc~StorageEngine~~
    }

    class CfManager {
        -Arc~TsdbDB~ db
        -CfConfig config
        -Mutex~Vec~String~~ known_cfs
        +new(db, config) CfManager
        +ensure_cf_for_date(date) Result~
        +cf_handle(cf_name) Option~CF~
        +cleanup_expired_cfs() Result~
    }

    class DimensionTable {
        -Arc~DB~ db
        -Mutex~HashMap~ tag_key_ids
        -Mutex~HashMap~ tag_key_reverse
        -Mutex~HashMap~ tag_value_ids
        -Mutex~HashMap~ tag_value_reverse
        -AtomicU64 next_key_id
        -AtomicU64 next_value_id
        +encode_tag_key(key) u32
        +decode_tag_key(id) Option~String~
        +encode_tag_value(key_id, value) u32
        +decode_tag_value(key_id, value_id) Option~String~
        +encode_tags(tags) Vec~u8~
        +decode_tags(data) Tags
    }

    DataPoint --> Tags
    DataPoint --> Fields
    Fields --> FieldValue
    StorageEngine --> RowKey : creates
    StorageEngine --> Qualifier : creates
    StorageEngine --> MergedBlock : reads/writes
    MergedBlock --> MergedField
    MergedField --> FieldValue
    MultiDbManager --> StorageEngine : manages
    StorageEngine --> CfManager : uses
    StorageEngine --> DimensionTable : uses
```

---

## 4. 索引层类图 (Index Layer Class Diagram)

```mermaid
classDiagram
    class IndexManager {
        +HashMap~String, SkipList~ time_index
        +HashMap~String, InvertedIndex~ tag_index
        +SeriesId next_series_id
        +HashMap~String, SeriesId~ series_cache
        +index_data_point(measurement, tags, ts, offset) SeriesId
        +query_by_time_range(measurement, start, end) Vec~
        +query_by_tags(measurement, filters) RoaringBitmap
        +get_series_id(series_key) Option~SeriesId~
        +serialize_all() Option~HashMap~
        +deserialize_entry(key, data)
    }

    class SkipList {
        +Vec~SkipNode~ nodes
        +usize head
        +usize max_level
        +usize len
        +u64 rng_state
        +new(max_level) SkipList
        +insert(key, block_offset)
        +range_query(start, end) Vec~
        +serialize() Vec~u8~
        +deserialize(data) Option~SkipList~
    }

    class SkipNode {
        +Timestamp key
        +Vec~u64~ block_offsets
        +Vec~Option~usize~~ forward
        +bool is_sentinel
    }

    class InvertedIndex {
        +HashMap~String, RoaringBitmap~ postings
        +HashMap~SeriesId, Vec~~ series_tags
        +add_series(series_id, tags)
        +remove_series(series_id)
        +query_exact(tag_key, tag_value) RoaringBitmap
        +query_intersection(filters) RoaringBitmap
        +query_union(filters) RoaringBitmap
        +all_series_ids() RoaringBitmap
        +serialize() Option~Vec~u8~~
        +deserialize(data) InvertedIndex
    }

    class IndexWAL {
        +PathBuf path
        +BufWriter~File~ writer
        +AtomicU64 sequence
        +AtomicU64 bytes_written
        +open(path) Result~IndexWAL~
        +append_insert(payload) Result~u64~
        +append_delete(payload) Result~u64~
        +append_checkpoint(payload) Result~u64~
        +replay(path) Result~Vec~WALEntry~~
        +rotate() Result~
    }

    class WALEntry {
        +u8 entry_type
        +u64 sequence
        +Vec~u8~ payload
    }

    IndexManager --> SkipList : time_index
    IndexManager --> InvertedIndex : tag_index
    IndexManager --> IndexWAL : uses
    SkipList --> SkipNode : contains
```

---

## 5. 写入路径时序图 (Write Path Sequence Diagram)

```mermaid
sequenceDiagram
    participant C as Client
    participant H as HTTP API
    participant S as TsdbServer
    participant E as StorageEngine
    participant RK as RowKey/Qualifier
    participant CF as CfManager
    participant D as DimensionTable
    participant R as RocksDB
    participant P as Pipeline
    participant A as Aggregator

    C->>H: POST /api/v1/write {db, measurement, tags, fields, ts}
    H->>S: handle_write(db, data)
    S->>S: get_database(db_name)

    S->>E: write(&DataPoint)

    par 键编码
        E->>RK: RowKey::from_data_point(dp)
        RK-->>E: RowKey {measurement, tags_hash, block_start}
        E->>D: compute_tag_signature(tags)
        D-->>E: tags_hash: u64
    and CF管理
        E->>CF: ensure_cf_for_date(date)
        CF->>R: create CF "data_YYYYMMDD" (if not exists)
        CF-->>E: cf_handle
    end

    loop 每个字段
        E->>RK: Qualifier::new(field_name, ts, block_start)
        RK-->>E: Qualifier {field_name, microsecond_offset}
        E->>E: key = [RowKey.encode() | 0x00 | Qualifier.encode()]
        E->>E: value = encode_field_value(field_value)
        E->>R: put_cf(&cf, &key, &value)
    end

    R-->>E: OK
    E-->>S: OK

    par 聚合通知
        S->>P: on_write(business, &dp)
        P->>A: accumulate(&dp)
        A->>A: 按 TimeDimension 分桶累加
        A-->>P: buffered
        P->>P: 检查 buffer_size / flush_interval
    end

    S-->>H: {"status": "ok"}
    H-->>C: 200 OK
```

---

## 6. Merge 写入路径时序图 (Merge Write Path)

```mermaid
sequenceDiagram
    participant C as Client
    participant E as StorageEngine
    participant RK as RowKey/Qualifier
    participant MO as MergeOperand
    participant R as RocksDB
    participant MOP as MergeOperator

    C->>E: write_merged(&DataPoint)

    E->>RK: RowKey::from_data_point(dp)
    RK-->>E: RowKey

    loop 每个字段
        E->>RK: Qualifier::new(field_name, ts, block_start)
        E->>MO: encode_merge_operand(field_name, offset, value)
        MO-->>E: operand bytes
        E->>R: merge_cf(&cf, &rk_bytes, &operand)
    end

    Note over R,MOP: RocksDB 内部触发合并
    R->>MOP: tsdb_block_merge(existing, operands)
    MOP->>MOP: 解码 existing → MergedBlock
    MOP->>MOP: 逐个应用 operands → upsert_field
    MOP->>MOP: 编码 MergedBlock → bytes
    MOP-->>R: 合并结果

    R-->>E: OK
    E-->>C: OK
```

---

## 7. 查询路径时序图 (Query Path Sequence Diagram)

```mermaid
sequenceDiagram
    participant C as Client
    participant H as HTTP API
    participant QE as QueryEngine
    participant P as SqlParser
    participant PL as QueryPlanner
    participant E as StorageEngine
    participant CF as CfManager
    participant R as RocksDB
    participant CB as ColumnarBatch
    participant VA as VectorizedEngine
    participant SA as SimdAggregator

    C->>H: GET /api/v1/query?sql=SELECT AVG(cpu) FROM sys WHERE host='s1'
    H->>QE: execute(sql, &db)

    QE->>P: parse(sql)
    P->>P: SQL → AST → ParsedQuery
    P-->>QE: ParsedQuery {measurement, select_fields, where_clause}

    QE->>PL: plan(&parsed)
    PL-->>QE: ExecutionPlan {scan_type, has_aggregations}

    alt FullScan / IndexScan
        QE->>E: read_range(measurement, tags, start, end)
        E->>CF: ensure_cf_for_date / cf_handle
        E->>R: prefix_iterator_cf(&cf, &prefix_key)

        loop 遍历 KV 对
            R-->>E: (key, value)
            E->>E: 检测 ValueFormat (Merged/Raw)
            alt Merged
                E->>E: MergedBlock::decode → to_data_points()
            else Raw
                E->>E: decode RowKey + Qualifier + FieldValue
            end
            E->>E: 过滤时间范围
        end

        E-->>QE: Vec<DataPoint>
        QE->>QE: 内存过滤 tag_filters
    end

    QE->>CB: from_data_points(&data_points)
    CB-->>QE: ColumnarBatch {columns, row_count}

    loop 每个 Aggregate 字段
        QE->>VA: execute_aggregate(&batch, field, func)
        VA->>SA: aggregate(batch, column_name, func)
        SA->>SA: 列式聚合计算
        SA-->>VA: FieldValue::Float(result)
        VA-->>QE: 聚合结果
    end

    QE-->>H: QueryResult {columns, rows}
    H-->>C: 200 JSON
```

---

## 8. 聚合管道时序图 (Aggregation Pipeline Sequence Diagram)

```mermaid
sequenceDiagram
    participant W as 写入线程
    participant P as LightAggregationPipeline
    participant A as Aggregator
    participant S as AggregationStore
    participant R as RocksDB (聚合)
    participant TG as TimeseriesGenerator
    participant CH as SvgRenderer

    W->>P: on_write(business, &dp)

    P->>A: accumulate(&dp)
    A->>A: 对每个 TimeDimension:
    A->>A: window_start = dim.align_timestamp(ts)
    A->>A: bucket_key = "measurement:dim:window"
    A->>A: 累加 float 字段值, 递增 count
    A-->>P: OK

    P->>P: buffer_count++
    P->>P: 检查 flush 条件

    alt buffer_count >= buffer_size
        P->>P: flush_bucket(key, business)
    else elapsed >= flush_interval
        P->>P: flush_all()
    end

    P->>A: finalize(measurement, dimension)
    A->>A: 遍历匹配分桶
    A->>A: SUM → AVG (÷ count)
    A-->>P: Vec<AggregationResult>

    P->>S: write_batch(&results)
    S->>S: 对每个 result:
    S->>S: cf = dimension.name()
    S->>S: key = "measurement|window_start"
    S->>S: value = JSON(values)
    S->>R: batch.put_cf(&cf, key, value)
    R-->>S: OK

    Note over TG,CH: 查询聚合结果生成图表
    TG->>S: query(dimension, measurement, start, end)
    S->>R: prefix_iterator_cf(&cf, prefix)
    R-->>S: KV 对
    S->>S: 反序列化 → Vec<AggregationResult>
    S-->>TG: results

    TG->>TG: 对每个字段: TimeSeries::add_point(ts, val)
    TG->>CH: render(&TimeSeriesChart)
    CH-->>TG: SVG string
```

---

## 9. 存储引擎组件图 (Storage Engine Component Diagram)

```mermaid
flowchart TB
    subgraph API["StorageEngine 公开API"]
        WRITE["write / write_batch"]
        WRITE_M["write_merged / write_merged_batch"]
        READ["read_range"]
        READ_M["get_point_merged"]
        COMP_W["write_compressed_block"]
        COMP_R["read_compressed_block / read_range_compressed"]
        CLEANUP["cleanup"]
        PERSIST["persist_index / restore_index"]
    end

    subgraph KeyEnc["键编码层"]
        RK["RowKey<br/>measurement | tags_hash 8B | block_start 8B"]
        Q["Qualifier<br/>field_name : offset 4B"]
        RK -->|0x00 分隔符| Q
    end

    subgraph ValFmt["值格式"]
        RAW["Raw 格式 type 1B | payload"]
        MERGED["MergedBlock 格式<br/>0xFEED magic | field_count | MergedField数组"]
        COMPRESSED["CompressedBlock 格式<br/>bincode 序列化 DataBlock"]
    end

    subgraph CFLayout["Column Family 布局"]
        META_CF["metadata CF 索引和元数据"]
        HOT_CF["data_YYYYMMDD HOT CF 0到7天 WriteBufferManager"]
        COLD_CF["data_YYYYMMDD COLD CF 8到30天 ZSTD压缩"]
    end

    subgraph MergePath["Merge 写入路径"]
        MO["MergeOperand field_name offset value"]
        MOP["tsdb_block_merge 合并同一RowKey字段"]
        MB_OUT["MergedBlock 多字段打包存储"]
    end

    subgraph CompressPath["压缩路径"]
        CODEC["BlockCodec 块级编解码"]
        GORILLA["GorillaEncoder float XOR 压缩比5到15倍"]
        DELTA["DeltaEncoder timestamp Delta 压缩比10倍"]
        DICT["DictionaryEncoder string 字典编码 压缩比3到10倍"]
    end

    WRITE --> KeyEnc
    WRITE --> RAW
    WRITE_M --> KeyEnc
    WRITE_M --> MO
    MO --> MOP
    MOP --> MERGED
    READ --> KeyEnc
    READ --> RAW
    READ --> MERGED
    READ_M --> MERGED
    COMP_W --> COMPRESSED
    COMP_R --> COMPRESSED
    COMP_W --> CODEC
    COMP_R --> CODEC
    CODEC --> GORILLA
    CODEC --> DELTA
    CODEC --> DICT

    WRITE --> CFLayout
    WRITE_M --> CFLayout
    READ --> CFLayout
    CLEANUP --> CFLayout
    PERSIST --> META_CF
```

---

## 10. RocksDB 数据布局图 (Storage Layout)

```mermaid
flowchart TB
    subgraph MainDB["主数据库 data_dir business"]
        subgraph MetaCF["metadata CF"]
            META_IDX_TIME["index:time:measurement<br/>SkipList 二进制序列化"]
            META_IDX_TAG["index:tag:measurement<br/>InvertedIndex 二进制序列化"]
            META_NEXT_ID["index:meta:next_series_id<br/>u64 LE 字节序"]
        end

        subgraph HotData["data_20260415 CF HOT 0到7天"]
            KV1["Key: cpu hash8B ts8B 0x00 usage:15000<br/>Val: type Float 0.75"]
            KV2["Key: cpu hash8B ts8B<br/>Val: MergedBlock 0xFEED 3字段"]
            KV3["Key: cpu hash8B ts8B 0xFF<br/>Val: CompressedBlock bincode"]
        end

        subgraph ColdData["data_20260408 CF COLD 8到30天 ZSTD"]
            KV_COLD["Key: cpu hash8B ts8B ...<br/>Val: 压缩后的 MergedBlock"]
        end
    end

    subgraph AggDB["聚合数据库 aggregation_data business"]
        subgraph HourCF["hour CF ZSTD压缩"]
            AGG_H["Key: measurement pipe window_start<br/>Val: JSON field value pairs"]
        end
        subgraph DayCF["day CF ZSTD压缩"]
            AGG_D["Key: measurement pipe window_start<br/>Val: JSON field value pairs"]
        end
        subgraph WeekCF["week CF ZSTD压缩"]
            AGG_W["Key: measurement pipe window_start<br/>Val: JSON field value pairs"]
        end
        subgraph MonthCF["month CF ZSTD压缩"]
            AGG_M["Key: measurement pipe window_start<br/>Val: JSON field value pairs"]
        end
    end
```

---

## 11. 协议 V2 编解码时序图 (Protocol V2 Sequence)

```mermaid
sequenceDiagram
    participant C as Client
    participant E as Envelope
    participant R as Request/Response
    participant M as MessagePack
    participant S as Server

    Note over C,S: === 请求编码 ===
    C->>R: Request::Write {db, measurement, tags, fields, ts}
    R->>M: rmp_serde::to_vec(&request)
    M-->>R: payload: Vec~u8~
    R->>E: encode_request(&request)
    E->>E: Envelope {magic=[0x54,0x53,0x44,0x42], version=2, crc32, payload}
    E->>E: crc32 = crc32fast::hash(&payload)
    E->>E: encode() → [magic|version|crc32|len|payload]
    E-->>C: bytes

    C->>S: TCP send(bytes)

    Note over C,S: === 请求解码 ===
    S->>E: decode(&bytes)
    E->>E: 验证 magic == [0x54,0x53,0x44,0x42]
    E->>E: 验证 version == 2
    E->>E: 验证 crc32 == crc32fast::hash(&payload)
    E-->>S: Envelope

    S->>R: decode_request(&envelope.payload)
    R->>M: rmp_serde::from_read(&payload)
    M-->>R: Request
    R-->>S: Request::Write {...}

    S->>S: process_request(request)

    Note over C,S: === 响应编码 ===
    S->>R: Response::QueryResult {columns, rows}
    S->>E: encode_response(&response)
    E->>M: rmp_serde::to_vec(&response)
    E->>E: wrap_v2(payload)
    E-->>S: bytes

    S->>C: TCP send(bytes)
```

---

## 12. 索引查询流程图 (Index Query Flow)

```mermaid
flowchart TD
    Q["查询请求<br/>measurement + filters"] --> HAS_TAG{有标签过滤?}

    HAS_TAG -->|是| TAG_Q["InvertedIndex.query_intersection(filters)"]
    HAS_TAG -->|否| TIME_Q["SkipList.range_query(start, end)"]

    TAG_Q --> BITMAP["RoaringBitmap 交集<br/>postings[k1=v1] & postings[k2=v2]"]
    BITMAP --> SERIES_IDS["匹配的 SeriesId 集合"]
    SERIES_IDS --> TAGS_LOOKUP["series_tags[series_id] → Tags"]
    TAGS_LOOKUP --> READ_RANGES["对每个 Tags:<br/>StorageEngine.read_range(measurement, tags, start, end)"]

    TIME_Q --> BLOCK_OFFSETS["Vec<(Timestamp, Vec<u64>)><br/>匹配的时间戳 + 块偏移"]
    BLOCK_OFFSETS --> READ_BLOCKS["按 block_offset 读取数据块"]

    READ_RANGES --> MERGE["合并结果"]
    READ_BLOCKS --> MERGE
    MERGE --> RESULT["Vec<DataPoint>"]

    style Q fill:#e3f2fd,stroke:#1976d2
    style RESULT fill:#e8f5e9,stroke:#388e3c
    style BITMAP fill:#fff3e0,stroke:#f57c00
```

---

## 13. 压缩算法分派图 (Compression Dispatch)

```mermaid
flowchart LR
    BLOCK["DataBlock"] --> TS["timestamps<br/>Vec<i64>"]
    BLOCK --> FLOAT["float 字段<br/>Vec<f64>"]
    BLOCK --> INT["int 字段<br/>Vec<i64>"]
    BLOCK --> STR["string 字段<br/>Vec<String>"]
    BLOCK --> BOOL["bool 字段<br/>Vec<bool>"]

    TS --> DELTA_E["DeltaEncoder<br/>Delta-of-Delta<br/>+ ZigZag + Varint<br/>压缩比 ~10:1"]
    FLOAT --> GORILLA_E["GorillaEncoder<br/>XOR 浮点压缩<br/>压缩比 ~5-15:1"]
    INT --> RAW_E["Big-Endian 原始<br/>8 bytes/值<br/>压缩比 1:1"]
    STR --> DICT_E["DictionaryEncoder<br/>字典编码<br/>压缩比 ~3-10:1"]
    BOOL --> BITPACK_E["位打包<br/>8 值/字节<br/>压缩比 8:1"]

    DELTA_E --> CB["CompressedBlock"]
    GORILLA_E --> CB
    RAW_E --> CB
    DICT_E --> CB
    BITPACK_E --> CB

    style BLOCK fill:#e3f2fd,stroke:#1976d2
    style CB fill:#e8f5e9,stroke:#388e3c
```
