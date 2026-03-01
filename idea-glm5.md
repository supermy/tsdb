# RocksDB时序数据库优化策略 - glm5版本

## 一、存储层优化

### 1.1 RowKey + Qualifier 机制

**核心设计：30秒定长块 + 微秒列偏移**

```
Key格式:  D:{measurement}:{series_id}:{block_start_ts}
Value格式: 30秒内的数据点紧凑存储，微秒偏移作为Qualifier
```

**优势：**
- 时序数据天然按时间有序，30秒块内压缩率极高
- 微秒级精度满足高频交易场景
- 减少Key数量，降低LSM-Tree层数

**实现要点：**
```c
// 块内数据结构
typedef struct {
    uint32_t offset_ms;    // 毫秒偏移
    uint32_t offset_us;    // 微秒偏移
    double   value;        // 数值
} data_point_t;

// 块头信息
typedef struct {
    uint64_t block_start;  // 块起始时间戳
    uint16_t count;        // 数据点数量
    uint8_t  compression;  // 压缩类型
    uint8_t  reserved;
} block_header_t;
```

### 1.2 冷热数据分离

**借鉴Kvrocks的ColumnFamily设计**

```
┌─────────────────────────────────────────────────────────┐
│                    RocksDB Instance                      │
├─────────────┬─────────────┬─────────────┬───────────────┤
│ CF:meta     │ CF:20240101 │ CF:20240102 │ CF:20240103   │
│ (元数据)    │ (冷数据)    │ (温数据)    │ (热数据)      │
│             │ ZSTD压缩    │ LZ4压缩     │ 无压缩        │
│             │ 无Compaction│ 自动Compact │ 自动Compact   │
└─────────────┴─────────────┴─────────────┴───────────────┘
```

**生命周期管理：**
```c
// 每日自动创建新CF
void create_daily_cf(rocksdb_t* db, const char* date) {
    rocksdb_column_family_handle_t* cf;
    rocksdb_options_t* cf_opts = rocksdb_options_create();
    rocksdb_options_set_compression(cf_opts, rocksdb_lz4_compression);
    
    char cf_name[32];
    snprintf(cf_name, sizeof(cf_name), "data_%s", date);
    rocksdb_create_column_family(db, cf_opts, cf_name, &cf);
}

// 冷数据CF降级
void downgrade_cold_cf(rocksdb_t* db, cf_handle_t* cf) {
    // 1. 切换压缩算法为ZSTD
    // 2. 关闭自动Compaction
    // 3. 手动执行一次全量Compaction
}

// 删除过期数据（秒级）
void drop_expired_cf(rocksdb_t* db, const char* cf_name) {
    rocksdb_drop_column_family(db, cf_name);
    // 无Compaction抖动，瞬间完成
}
```

### 1.3 Key编码优化

**当前实现 vs 优化方案**

| 维度 | 当前实现 | 优化方案 |
|------|----------|----------|
| Key格式 | `D:cpu:123:1700000000000000000` | `D:cpu:123:1700000000:30000` |
| Key长度 | ~45字节 | ~30字节 |
| 时间精度 | 纳秒 | 秒+毫秒偏移 |
| 存储密度 | 低 | 高 |

**优化后的Key设计：**
```c
// 二进制Key结构（节省30%空间）
typedef struct __attribute__((packed)) {
    uint8_t  prefix;        // 'D' = 1字节
    uint8_t  meas_len;      // measurement长度 = 1字节
    char     measurement[32]; // 动态长度
    uint64_t series_id;     // 8字节
    uint32_t block_ts;      // 块起始秒 = 4字节
    uint16_t block_offset;  // 块内偏移 = 2字节
} compact_key_t;
```

---

## 二、索引层优化

### 2.1 内存索引结构

**当前问题：** 每次写入都更新索引，开销大

**优化方案：** 批量索引更新 + Bloom Filter加速

```c
typedef struct {
    // 主索引：series_id -> series_info
    rocksdb_t* series_index;  // 持久化到独立CF
    
    // 布隆过滤器：快速判断series是否存在
    rocksdb_filterpolicy_t* bloom_filter;
    
    // 写缓冲：批量更新索引
    rtsdb_series_info_t* write_buffer[4096];
    size_t buffer_count;
    pthread_mutex_t buffer_lock;
} optimized_index_t;

// 批量更新索引
void flush_index_buffer(optimized_index_t* idx) {
    rocksdb_writebatch_t* batch = rocksdb_writebatch_create();
    for (size_t i = 0; i < idx->buffer_count; i++) {
        // 序列化series_info
        // rocksdb_writebatch_put(batch, ...)
    }
    rocksdb_write(idx->series_index, wopts, batch, NULL);
    idx->buffer_count = 0;
}
```

### 2.2 时间范围索引

**为每个series维护时间范围元数据**

```c
typedef struct {
    uint64_t series_id;
    uint64_t min_ts;
    uint64_t max_ts;
    uint64_t point_count;
    uint32_t block_count;
    uint8_t  compression_type;
} series_meta_t;

// 查询时快速过滤
bool series_overlap_range(series_meta_t* meta, uint64_t start, uint64_t end) {
    return !(meta->max_ts < start || meta->min_ts > end);
}
```

---

## 三、查询层优化

### 3.1 向量化查询

**当前：** 逐条迭代
**优化：** 批量读取 + SIMD聚合

```c
// 向量化聚合
void aggregate_simd(double* values, size_t count, double* result) {
    __m256d sum = _mm256_setzero_pd();
    for (size_t i = 0; i < count; i += 4) {
        __m256d v = _mm256_loadu_pd(&values[i]);
        sum = _mm256_add_pd(sum, v);
    }
    // 水平求和
    double temp[4];
    _mm256_storeu_pd(temp, sum);
    *result = temp[0] + temp[1] + temp[2] + temp[3];
}
```

### 3.2 查询缓存

```c
typedef struct {
    char key[256];        // measurement + range hash
    rtsdb_result_set_t* result;
    uint64_t expire_ts;
    uint32_t hit_count;
} query_cache_entry_t;

typedef struct {
    query_cache_entry_t* entries;
    size_t capacity;
    size_t count;
    pthread_rwlock_t lock;
} query_cache_t;

// LRU缓存淘汰
void cache_put(query_cache_t* cache, const char* key, rtsdb_result_set_t* result);
rtsdb_result_set_t* cache_get(query_cache_t* cache, const char* key);
```

---

## 四、压缩优化

### 4.1 时序专用压缩算法

**Gorilla压缩（Facebook TSDB）**

```c
// 时间戳压缩：XOR + 变长编码
typedef struct {
    int64_t prev_ts;
    int32_t prev_delta;
} ts_compressor_t;

// 值压缩：XOR + 前导零 + 尾随零
typedef struct {
    double prev_val;
    uint8_t prev_leading;   // 前导零
    uint8_t prev_trailing;  // 尾随零
} val_compressor_t;

// 压缩率：10:1 ~ 20:1
```

### 4.2 分级压缩策略

| 数据年龄 | CF类型 | 压缩算法 | 压缩级别 |
|----------|--------|----------|----------|
| 0-7天 | 热数据 | 无/LZ4 | 1 |
| 7-30天 | 温数据 | LZ4 | 4 |
| 30天+ | 冷数据 | ZSTD | 9 |

---

## 五、业务支持

### 5.1 多业务隔离

```
┌─────────────────────────────────────────────────────────┐
│                    业务隔离架构                          │
├─────────────┬─────────────┬─────────────┬───────────────┤
│ DB:stock    │ DB:iot      │ DB:payment  │ DB:sms        │
│ 股票行情    │ 物联网      │ 支付流水    │ 短信下发      │
│ 高频写入    │ 低频写入    │ 事务写入    │ 批量写入      │
│ 7天保留     │ 365天保留   │ 永久保留    │ 90天保留      │
└─────────────┴─────────────┴─────────────┴───────────────┘
```

### 5.2 维度表编码

```c
// Tag编码表
typedef struct {
    char tag_key[64];
    char tag_value[256];
    uint32_t encoded_id;  // 编码后的ID
} tag_encoding_t;

// 维度表存储
rocksdb_t* dimension_db;  // 独立DB存储维度映射

// 编码示例
// host=server01 -> 0x0001
// region=us-east -> 0x0002
// 组合编码: 0x00010002
```

### 5.3 轻度汇总

```
明细数据写入流程:
    │
    ├─→ 写入明细DB (rocksdb-glm5)
    │
    └─→ 异步队列 (ZeroMQ)
            │
            └─→ 消费者进程
                    │
                    ├─→ 按小时聚合 → 汇总DB:hourly
                    ├─→ 按天聚合   → 汇总DB:daily
                    └─→ 按月聚合   → 汇总DB:monthly
```

**汇总数据存储：**
```c
// 汇总Key格式
// H:cpu:hourly:2024010110  (小时汇总)
// D:cpu:daily:20240101     (天汇总)
// M:cpu:monthly:202401     (月汇总)

typedef struct {
    double sum;
    double min;
    double max;
    double avg;
    uint64_t count;
    uint64_t first_ts;
    uint64_t last_ts;
} agg_summary_t;
```

---

## 六、性能目标

| 指标 | 当前性能 | 优化目标 |
|------|----------|----------|
| 写入吞吐 | 50K pts/s | 500K pts/s |
| 查询延迟 | 100ms | 10ms |
| 压缩率 | 2:1 | 10:1 |
| 存储成本 | 100% | 30% |

---

## 七、实施路线图

### Phase 1: 存储优化（2周）
- [ ] 实现30秒定长块存储
- [ ] 实现ColumnFamily冷热分离
- [ ] 优化Key编码

### Phase 2: 索引优化（1周）
- [ ] 批量索引更新
- [ ] Bloom Filter加速
- [ ] 时间范围索引

### Phase 3: 查询优化（1周）
- [ ] 向量化聚合
- [ ] 查询缓存
- [ ] 并行查询

### Phase 4: 压缩优化（1周）
- [ ] Gorilla压缩算法
- [ ] 分级压缩策略
- [ ] 压缩率监控

### Phase 5: 业务功能（2周）
- [ ] 多业务隔离
- [ ] 维度表编码
- [ ] 轻度汇总
