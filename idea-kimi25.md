# RocksDB时序数据库优化策略 - kimi25版本

## 一、架构定位

kimi25定位为**极简轻量级时序数据库**，核心设计理念：
- **代码极简**：单文件实现，< 300行核心代码
- **零依赖**：仅依赖RocksDB，无其他外部库
- **易用优先**：API简洁，上手即用
- **适合场景**：嵌入式、IoT设备、边缘计算、原型验证

---

## 二、存储层优化

### 2.1 极简Key设计

**当前实现：** 字符串Key `cpu_usage_123_1700000000000000000`

**优化方案：紧凑型Key**

```c
// 优化前：字符串Key ~45字节
// "cpu_usage_123_1700000000000000000"

// 优化后：二进制Key ~20字节
typedef struct __attribute__((packed)) {
    uint8_t  type;           // 数据类型标识 'D'
    uint16_t meas_len;       // measurement长度
    char     measurement[32]; // 变长
    uint64_t series_id;      // 8字节
    uint64_t timestamp;      // 8字节（秒级）
} compact_key_t;

// Key构造
void make_compact_key(char* buf, size_t* len, 
                      const char* meas, uint64_t sid, uint64_t ts) {
    compact_key_t key;
    key.type = 'D';
    key.meas_len = strlen(meas);
    memcpy(key.measurement, meas, key.meas_len);
    key.series_id = sid;
    key.timestamp = ts / 1000000000ULL;  // 纳秒转秒
    
    *len = sizeof(key.type) + sizeof(key.meas_len) + key.meas_len + 
           sizeof(key.series_id) + sizeof(key.timestamp);
    memcpy(buf, &key, *len);
}
```

**优势：**
- Key大小减少 **55%**（45字节 → 20字节）
- 减少RocksDB内存占用
- 提升查询效率

### 2.2 批量写入优化

**当前实现：** 单点写入

**优化方案：WriteBatch + 自动批量**

```c
typedef struct {
    point_t* buffer;
    size_t count;
    size_t capacity;
    
    // 自动刷新阈值
    size_t batch_size;
    uint64_t flush_interval_ms;
    
    pthread_mutex_t lock;
    pthread_cond_t cond;
    pthread_t flush_thread;
    bool running;
} write_buffer_t;

// 初始化自动批量写入
write_buffer_t* write_buffer_create(size_t batch_size, uint64_t interval_ms) {
    write_buffer_t* wb = calloc(1, sizeof(write_buffer_t));
    wb->buffer = calloc(batch_size, sizeof(point_t));
    wb->capacity = batch_size;
    wb->batch_size = batch_size;
    wb->flush_interval_ms = interval_ms;
    pthread_mutex_init(&wb->lock, NULL);
    pthread_cond_init(&wb->cond, NULL);
    
    // 启动后台刷新线程
    wb->running = true;
    pthread_create(&wb->flush_thread, NULL, flush_worker, wb);
    
    return wb;
}

// 后台刷新线程
void* flush_worker(void* arg) {
    write_buffer_t* wb = (write_buffer_t*)arg;
    
    while (wb->running) {
        struct timespec ts;
        clock_gettime(CLOCK_REALTIME, &ts);
        ts.tv_nsec += wb->flush_interval_ms * 1000000;
        
        pthread_mutex_lock(&wb->lock);
        pthread_cond_timedwait(&wb->cond, &wb->lock, &ts);
        
        if (wb->count > 0) {
            flush_batch(wb);
        }
        pthread_mutex_unlock(&wb->lock);
    }
    return NULL;
}

// 批量写入API
int kimi_write_buffered(rkimi_db_t* db, const rkimi_point_t* point) {
    write_buffer_t* wb = db->write_buffer;
    
    pthread_mutex_lock(&wb->lock);
    
    // 复制到缓冲区
    wb->buffer[wb->count++] = *point;
    
    // 达到阈值，立即刷新
    if (wb->count >= wb->batch_size) {
        flush_batch(wb);
        pthread_cond_signal(&wb->cond);
    }
    
    pthread_mutex_unlock(&wb->lock);
    return 0;
}

// 执行批量写入
int flush_batch(write_buffer_t* wb) {
    rocksdb_writebatch_t* batch = rocksdb_writebatch_create();
    
    for (size_t i = 0; i < wb->count; i++) {
        char key[64], value[32];
        make_compact_key(key, &key_len, wb->buffer[i].measurement, 
                        hash_series(&wb->buffer[i]), wb->buffer[i].timestamp);
        snprintf(value, sizeof(value), "%.10f", wb->buffer[i].value);
        
        rocksdb_writebatch_put(batch, key, key_len, value, strlen(value));
    }
    
    rocksdb_write(wb->db, wb->wopts, batch, NULL);
    rocksdb_writebatch_destroy(batch);
    
    wb->count = 0;
    return 0;
}
```

**性能提升：**
- 写入吞吐提升 **5-10倍**
- 减少RocksDB Compaction压力

### 2.3 预写日志优化

```c
// 禁用WAL提升写入性能（牺牲一定持久性）
void kimi_write_no_wal(rkimi_db_t* db, const rkimi_point_t* point) {
    rocksdb_writeoptions_t* wopts = rocksdb_writeoptions_create();
    rocksdb_writeoptions_disable_wal(wopts, 1);  // 禁用WAL
    
    char key[64], value[32];
    make_key(key, sizeof(key), point->measurement, 
             hash_series(point), point->timestamp);
    snprintf(value, sizeof(value), "%.10f", point->value);
    
    rocksdb_put(db->db, wopts, key, strlen(key), value, strlen(value), NULL);
    rocksdb_writeoptions_destroy(wopts);
}

// 同步刷盘（保证持久性）
void kimi_sync(rkimi_db_t* db) {
    rocksdb_flushoptions_t* fopts = rocksdb_flushoptions_create();
    rocksdb_flush(db->db, fopts, NULL);
    rocksdb_flushoptions_destroy(fopts);
}
```

---

## 三、查询层优化

### 3.1 前缀扫描优化

**当前实现：** 遍历所有数据

**优化方案：前缀 + 范围限制**

```c
// 优化后的范围查询
int kimi_query_optimized(rkimi_db_t* db, const char* measurement,
                         const rkimi_range_t* range, size_t limit,
                         rkimi_result_t* result) {
    
    rocksdb_readoptions_t* ropts = rocksdb_readoptions_create();
    
    // 设置迭代器上下界，减少扫描范围
    if (range) {
        char lower_key[64], upper_key[64];
        size_t lower_len, upper_len;
        
        // 构造下界Key
        make_compact_key(lower_key, &lower_len, measurement, 0, range->start);
        rocksdb_readoptions_set_iterate_lower_bound(ropts, lower_key, lower_len);
        
        // 构造上界Key
        make_compact_key(upper_key, &upper_len, measurement, UINT64_MAX, range->end);
        rocksdb_readoptions_set_iterate_upper_bound(ropts, upper_key, upper_len);
    }
    
    rocksdb_iterator_t* it = rocksdb_create_iterator(db->db, ropts);
    
    // 前缀定位
    char prefix[32];
    size_t prefix_len;
    make_compact_key(prefix, &prefix_len, measurement, 0, 0);
    
    rocksdb_iter_seek(it, prefix, prefix_len);
    
    // 限制扫描数量
    size_t scanned = 0;
    size_t max_scan = limit * 10;  // 最多扫描10倍limit
    
    while (rocksdb_iter_valid(it) && result->count < limit && scanned < max_scan) {
        // 处理数据...
        scanned++;
        rocksdb_iter_next(it);
    }
    
    rocksdb_iter_destroy(it);
    rocksdb_readoptions_destroy(ropts);
    return 0;
}
```

### 3.2 结果预分配

```c
// 预分配结果内存，避免频繁realloc
rkimi_result_t* result_create(size_t initial_capacity) {
    rkimi_result_t* res = calloc(1, sizeof(rkimi_result_t));
    res->capacity = initial_capacity > 0 ? initial_capacity : 1024;
    res->points = calloc(res->capacity, sizeof(rkimi_point_t));
    return res;
}

// 批量扩容（2倍增长）
int result_ensure_capacity(rkimi_result_t* res, size_t required) {
    if (required <= res->capacity) return 0;
    
    size_t new_cap = res->capacity;
    while (new_cap < required) {
        new_cap *= 2;
    }
    
    rkimi_point_t* new_pts = realloc(res->points, new_cap * sizeof(rkimi_point_t));
    if (!new_pts) return -1;
    
    res->points = new_pts;
    res->capacity = new_cap;
    return 0;
}
```

### 3.3 简单缓存

```c
// 极简LRU缓存（固定大小）
#define CACHE_SIZE 1024

typedef struct {
    uint64_t key_hash;           // Key的hash
    rkimi_result_t* result;      // 缓存结果
    uint64_t timestamp;          // 缓存时间
    uint32_t hit_count;          // 命中次数
} cache_entry_t;

typedef struct {
    cache_entry_t entries[CACHE_SIZE];
    pthread_mutex_t lock;
    uint64_t total_hits;
    uint64_t total_misses;
} simple_cache_t;

// 计算查询Key的hash
uint64_t query_hash(const char* measurement, const rkimi_range_t* range) {
    uint64_t h = 14695981039346656037ULL;
    const uint8_t* bytes = (const uint8_t*)measurement;
    for (size_t i = 0; i < strlen(measurement); i++) {
        h ^= bytes[i];
        h *= 1099511628211ULL;
    }
    h ^= range->start;
    h ^= range->end;
    return h;
}

// 缓存查找
rkimi_result_t* cache_get(simple_cache_t* cache, uint64_t hash) {
    pthread_mutex_lock(&cache->lock);
    
    uint32_t idx = hash % CACHE_SIZE;
    cache_entry_t* entry = &cache->entries[idx];
    
    if (entry->key_hash == hash && entry->result != NULL) {
        entry->hit_count++;
        cache->total_hits++;
        pthread_mutex_unlock(&cache->lock);
        return entry->result;
    }
    
    cache->total_misses++;
    pthread_mutex_unlock(&cache->lock);
    return NULL;
}

// 缓存写入
void cache_put(simple_cache_t* cache, uint64_t hash, rkimi_result_t* result) {
    pthread_mutex_lock(&cache->lock);
    
    uint32_t idx = hash % CACHE_SIZE;
    cache_entry_t* entry = &cache->entries[idx];
    
    // 释放旧缓存
    if (entry->result) {
        rkimi_result_free(entry->result);
    }
    
    // 写入新缓存（复制结果）
    entry->key_hash = hash;
    entry->result = result_copy(result);
    entry->timestamp = time(NULL);
    entry->hit_count = 0;
    
    pthread_mutex_unlock(&cache->lock);
}
```

---

## 四、配置优化

### 4.1 RocksDB参数调优

```c
rocksdb_options_t* kimi_optimized_options(void) {
    rocksdb_options_t* opts = rocksdb_options_create();
    
    // 基础配置
    rocksdb_options_set_create_if_missing(opts, 1);
    
    // 内存配置
    rocksdb_options_set_write_buffer_size(opts, 32 * 1024 * 1024);  // 32MB
    rocksdb_options_set_max_write_buffer_number(opts, 3);
    rocksdb_options_set_target_file_size_base(opts, 32 * 1024 * 1024);
    
    // Level配置
    rocksdb_options_set_max_bytes_for_level_base(opts, 128 * 1024 * 1024);
    rocksdb_options_set_max_bytes_for_level_multiplier(opts, 10);
    rocksdb_options_set_level0_file_num_compaction_trigger(opts, 4);
    rocksdb_options_set_level0_slowdown_writes_trigger(opts, 20);
    rocksdb_options_set_level0_stop_writes_trigger(opts, 36);
    
    // 压缩配置
    rocksdb_options_set_compression(opts, rocksdb_lz4_compression);
    
    // 布隆过滤器
    rocksdb_filterpolicy_t* bloom = rocksdb_filterpolicy_create_bloom(10);
    rocksdb_options_set_filter_policy(opts, bloom);
    
    // 块缓存
    rocksdb_cache_t* cache = rocksdb_cache_create_lru(64 * 1024 * 1024);  // 64MB
    rocksdb_options_set_cache(opts, cache);
    
    // 表配置
    rocksdb_block_based_table_options_t* table_opts = rocksdb_block_based_options_create();
    rocksdb_block_based_options_set_block_size(table_opts, 16 * 1024);  // 16KB
    rocksdb_options_set_block_based_table_factory(opts, table_opts);
    
    return opts;
}
```

### 4.2 运行时配置

```c
typedef struct {
    // 写入配置
    size_t batch_size;              // 批量大小
    uint64_t flush_interval_ms;     // 刷新间隔
    bool enable_wal;                // 是否启用WAL
    
    // 查询配置
    size_t query_cache_size;        // 查询缓存大小
    size_t max_scan_multiplier;     // 最大扫描倍数
    
    // 存储配置
    size_t write_buffer_mb;         // 写缓冲区大小
    size_t cache_size_mb;           // 块缓存大小
    int compression_level;          // 压缩级别
} kimi_config_t;

// 默认配置（轻量级）
kimi_config_t kimi_default_config(void) {
    return (kimi_config_t){
        .batch_size = 1000,
        .flush_interval_ms = 100,
        .enable_wal = true,
        .query_cache_size = 1024,
        .max_scan_multiplier = 10,
        .write_buffer_mb = 32,
        .cache_size_mb = 64,
        .compression_level = 1
    };
}

// 高性能配置（牺牲持久性）
kimi_config_t kimi_high_perf_config(void) {
    return (kimi_config_t){
        .batch_size = 5000,
        .flush_interval_ms = 50,
        .enable_wal = false,
        .query_cache_size = 4096,
        .max_scan_multiplier = 5,
        .write_buffer_mb = 64,
        .cache_size_mb = 128,
        .compression_level = 0
    };
}
```

---

## 五、业务功能

### 5.1 数据保留策略

```c
// 简单的TTL删除
int kimi_delete_expired(rkimi_db_t* db, uint64_t before_ts) {
    rocksdb_iterator_t* it = rocksdb_create_iterator(db->db, db->ropts);
    rocksdb_writebatch_t* batch = rocksdb_writebatch_create();
    
    rocksdb_iter_seek_to_first(it);
    size_t deleted = 0;
    
    while (rocksdb_iter_valid(it)) {
        size_t key_len;
        const char* key = rocksdb_iter_key(it, &key_len);
        
        // 解析时间戳
        uint64_t ts = extract_timestamp(key, key_len);
        
        if (ts < before_ts) {
            rocksdb_writebatch_delete(batch, key, key_len);
            deleted++;
            
            // 批量提交
            if (deleted >= 10000) {
                rocksdb_write(db->db, db->wopts, batch, NULL);
                rocksdb_writebatch_clear(batch);
                deleted = 0;
            }
        }
        
        rocksdb_iter_next(it);
    }
    
    // 提交剩余
    if (deleted > 0) {
        rocksdb_write(db->db, db->wopts, batch, NULL);
    }
    
    rocksdb_writebatch_destroy(batch);
    rocksdb_iter_destroy(it);
    return 0;
}
```

### 5.2 简单聚合

```c
// 流式聚合（不缓存所有数据）
typedef struct {
    double sum;
    double min;
    double max;
    uint64_t count;
    double first;
    double last;
    uint64_t first_ts;
    uint64_t last_ts;
} streaming_agg_t;

int kimi_agg_streaming(rkimi_db_t* db, const char* measurement,
                       const rkimi_range_t* range, rkimi_agg_t agg,
                       rkimi_agg_result_t* result) {
    
    streaming_agg_t agg_state = {
        .sum = 0, .min = INFINITY, .max = -INFINITY,
        .count = 0, .first = 0, .last = 0
    };
    
    rocksdb_iterator_t* it = rocksdb_create_iterator(db->db, db->ropts);
    
    char prefix[32];
    size_t prefix_len;
    make_compact_key(prefix, &prefix_len, measurement, 0, 0);
    rocksdb_iter_seek(it, prefix, prefix_len);
    
    while (rocksdb_iter_valid(it)) {
        size_t key_len, val_len;
        const char* key = rocksdb_iter_key(it, &key_len);
        const char* val = rocksdb_iter_value(it, &val_len);
        
        // 检查前缀
        if (key_len < prefix_len || memcmp(key, prefix, prefix_len) != 0) {
            break;
        }
        
        // 解析时间戳和值
        uint64_t ts = extract_timestamp(key, key_len);
        double v = atof(val);
        
        if (!range || (ts >= range->start && ts <= range->end)) {
            // 流式更新聚合状态
            agg_state.sum += v;
            if (v < agg_state.min) agg_state.min = v;
            if (v > agg_state.max) agg_state.max = v;
            if (agg_state.count == 0) {
                agg_state.first = v;
                agg_state.first_ts = ts;
            }
            agg_state.last = v;
            agg_state.last_ts = ts;
            agg_state.count++;
        }
        
        rocksdb_iter_next(it);
    }
    
    rocksdb_iter_destroy(it);
    
    // 填充结果
    result->count = agg_state.count;
    switch (agg) {
        case RKIMI_AGG_COUNT: result->value = agg_state.count; break;
        case RKIMI_AGG_SUM:   result->value = agg_state.sum; break;
        case RKIMI_AGG_AVG:   result->value = agg_state.sum / agg_state.count; break;
        case RKIMI_AGG_MIN:   result->value = agg_state.min; break;
        case RKIMI_AGG_MAX:   result->value = agg_state.max; break;
        case RKIMI_AGG_FIRST: result->value = agg_state.first; break;
        case RKIMI_AGG_LAST:  result->value = agg_state.last; break;
    }
    
    return 0;
}
```

---

## 六、性能目标

| 指标 | 当前 | 优化目标 | 优化手段 |
|------|------|----------|----------|
| 代码行数 | ~250行 | < 500行 | 保持极简 |
| 写入吞吐 | 50K/s | 300K/s | 批量写入 + 无WAL |
| 查询延迟 | 100ms | 20ms | 前缀扫描 + 缓存 |
| 内存占用 | 低 | 极低 | 精简缓存 |
| 启动速度 | 快 | 极快 | 无预加载 |

---

## 七、实施计划

### Phase 1: Key优化（2天）
- [ ] 实现二进制紧凑型Key
- [ ] 更新序列化/反序列化
- [ ] 测试Key大小变化

### Phase 2: 批量写入（3天）
- [ ] 实现WriteBuffer结构
- [ ] 后台刷新线程
- [ ] WAL开关控制

### Phase 3: 查询优化（2天）
- [ ] 迭代器边界设置
- [ ] 简单LRU缓存
- [ ] 流式聚合

### Phase 4: 配置优化（2天）
- [ ] RocksDB参数调优
- [ ] 运行时配置API
- [ ] 性能测试

### Phase 5: 功能完善（1天）
- [ ] 数据保留策略
- [ ] 简单聚合函数
- [ ] 文档更新

---

## 八、API设计

```c
// 核心API（保持极简）

// 打开/关闭
rkimi_db_t* kimi_open(const char* path);
rkimi_db_t* kimi_open_with_config(const char* path, const kimi_config_t* config);
void kimi_close(rkimi_db_t* db);

// 写入
int kimi_write(rkimi_db_t* db, const rkimi_point_t* point);
int kimi_write_batch(rkimi_db_t* db, const rkimi_point_t* points, size_t count);
void kimi_flush(rkimi_db_t* db);

// 查询
int kimi_query(rkimi_db_t* db, const char* measurement,
               const rkimi_range_t* range, size_t limit,
               rkimi_result_t* result);
int kimi_agg(rkimi_db_t* db, const char* measurement,
             const rkimi_range_t* range, rkimi_agg_t agg,
             rkimi_agg_result_t* result);

// 管理
int kimi_delete_expired(rkimi_db_t* db, uint64_t before_ts);
int kimi_compact(rkimi_db_t* db);

// 工具
rkimi_point_t* kimi_point_new(const char* measurement, uint64_t ts);
rkimi_point_t* kimi_tag(rkimi_point_t* p, const char* key, const char* value);
rkimi_point_t* kimi_val(rkimi_point_t* p, double v);
void kimi_result_free(rkimi_result_t* r);
```

---

## 九、与其他版本对比

| 特性 | kimi25 | minimax25 | glm5 |
|------|--------|-----------|------|
| 代码量 | ~300行 | ~1000行 | ~3000行 |
| 架构 | 单文件极简 | 单文件增强 | 多模块 |
| 依赖 | RocksDB | RocksDB | RocksDB |
| 批量写入 | 自动缓冲 | 自动缓冲 | 手动+自动 |
| 缓存 | 简单LRU | 多级缓存 | 复杂缓存 |
| 压缩 | LZ4 | Delta+LZ4 | Gorilla+ZSTD |
| 目标场景 | IoT/边缘 | 中小规模 | 大规模生产 |
| 性能 | 300K/s | 200K/s | 500K/s |

kimi25版本追求**极简极致**，在保持代码简洁的同时，通过关键优化点提升性能。
