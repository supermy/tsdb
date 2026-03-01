# RocksDB时序数据库优化策略 - minimax25版本

## 一、架构定位

minimax25定位为**轻量级高性能时序数据库**，采用简洁架构：
- 单文件实现，代码量少，易于维护
- 内存为主 + 持久化
- 适合中小规模数据场景（GB级）

---

## 二、存储层优化

### 2.1 块级存储机制

**当前问题：** 逐点存储，Key数量多

**优化方案：时间块聚合存储**

```c
// 块级存储结构
typedef struct {
    uint64_t block_start;      // 块起始时间（秒）
    uint32_t block_size;       // 块大小（秒）
    uint32_t capacity;        // 容量
    uint32_t count;           // 当前数量
    
    // 时间序列紧凑存储
    uint32_t* timestamps;     // 相对偏移（4字节/点）
    double* values;           // 数值
    
    bool dirty;               // 脏标记
} time_block_t;

// 块管理器
typedef struct {
    time_block_t** blocks;
    size_t block_count;
    size_t block_capacity;
    uint32_t block_seconds;   // 块大小（默认60秒）
} block_manager_t;

// 块写入
int block_write(block_manager_t* mgr, uint64_t ts, double val) {
    uint64_t block_start = (ts / mgr->block_seconds) * mgr->block_seconds;
    time_block_t* blk = get_or_create_block(mgr, block_start);
    
    // 存储相对偏移，节省空间
    blk->timestamps[blk->count] = (uint32_t)(ts - block_start);
    blk->values[blk->count] = val;
    blk->count++;
    blk->dirty = true;
    return 0;
}
```

**优势：**
- 减少Key数量：1个Key代替N个
- 块内紧凑存储，压缩率高
- 批量刷盘，减少I/O次数

### 2.2 ColumnFamily冷热分离

**借鉴Kvrocks设计，按日期分CF**

```c
// CF命名规则
// CF: default  - 当前热数据
// CF: 20240101 - 2024年1月1日数据
// CF: 20240102 - 2024年1月2日数据

typedef struct {
    char db_path[256];
    rocksdb_t* db;
    
    // CF管理
    rocksdb_column_family_handle_t* default_cf;
    char current_date[16];           // 当前日期
    time_t last_cf_check;            // 上次检查时间
    
    // 配置
    int hot_data_days;               // 热数据保留天数
    int compression_type;            // 压缩类型
} multi_cf_manager_t;

// 每日自动创建新CF
int create_daily_cf(multi_cf_manager_t* mgr) {
    char date[16];
    strftime(date, sizeof(date), "%Y%m%d", localtime(&(time_t){time(NULL)}));
    
    if (strcmp(mgr->current_date, date) != 0) {
        char cf_name[32];
        snprintf(cf_name, sizeof(cf_name), "data_%s", date);
        
        rocksdb_options_t* opts = rocksdb_options_create();
        rocksdb_options_set_compression(opts, rocksdb_lz4_compression);
        
        rocksdb_column_family_handle_t* cf;
        rocksdb_create_column_family(mgr->db, opts, cf_name, &cf);
        
        strcpy(mgr->current_date, date);
    }
    return 0;
}

// 删除过期CF（秒级）
int drop_expired_cf(multi_cf_manager_t* mgr, int days) {
    char date[16];
    time_t expire = time(NULL) - (days * 86400);
    strftime(date, sizeof(date), "%Y%m%d", localtime(&expire));
    
    char cf_name[32];
    snprintf(cf_name, sizeof(cf_name), "data_%s", date);
    
    rocksdb_drop_column_family_by_name(mgr->db, cf_name, NULL);
    return 0;
}
```

### 2.3 Key编码优化

**当前实现：** `cpu_usage_123_1700000000000000000`

**优化方案：** 二进制复合Key

```c
// 二进制Key结构
typedef struct __attribute__((packed)) {
    uint8_t  type;           // 'D'=数据, 'I'=索引
    uint64_t series_id;     // series ID (8字节)
    uint32_t timestamp;     // 时间戳秒数 (4字节)
    uint16_t offset;        // 块内偏移 (2字节)
} binary_key_t;

// 序列化/反序列化
void serialize_key(char* buf, size_t* len, const binary_key_t* key) {
    *len = sizeof(binary_key_t);
    memcpy(buf, key, *len);
}

void deserialize_key(const char* buf, size_t len, binary_key_t* key) {
    memcpy(key, buf, sizeof(binary_key_t));
}

// Key对比（用于RocksDB排序）
int key_compare(const char* a, size_t alen, const char* b, size_t blen) {
    const binary_key_t* ka = (const binary_key_t*)a;
    const binary_key_t* kb = (const binary_key_t*)b;
    
    if (ka->type != kb->type) return ka->type - kb->type;
    if (ka->series_id != kb->series_id) return (ka->series_id > kb->series_id) ? 1 : -1;
    if (ka->timestamp != kb->timestamp) return (ka->timestamp > kb->timestamp) ? 1 : -1;
    return ka->offset - kb->offset;
}
```

---

## 三、索引层优化

### 3.1 多级索引架构

```c
// 一级索引：内存Hash表（热点数据）
typedef struct {
    uint64_t series_id;
    char measurement[128];
    
    // 时间范围（用于快速剪枝）
    uint64_t min_ts;
    uint64_t max_ts;
    
    // 块信息
    time_block_t* current_block;
    uint32_t block_count;
    
    // 统计信息
    uint64_t point_count;
    
    // 链表指针
    struct series_entry* hash_next;
} series_index_t;

// 全局Hash表
typedef struct {
    series_index_t** buckets;
    size_t bucket_count;
    size_t series_count;
    pthread_rwlock_t rwlock;
} index_hash_t;

// 二级索引：持久化索引（磁盘）
typedef struct {
    char idx_path[256];
    rocksdb_t* idx_db;
} persist_index_t;
```

### 3.2 索引预加载与延迟加载

```c
// 启动时只加载热点series
int index_load_hot(index_hash_t* idx, persist_index_t* pidx, int limit) {
    // 从持久化索引读取最近的limit个series
    rocksdb_iterator_t* it = rocksdb_create_iterator(pidx->idx_db, NULL);
    rocksdb_iter_seek_to_last(it);
    
    for (int i = 0; i < limit && rocksdb_iter_valid(it); i++) {
        // 加载到内存Hash表
        series_index_t* entry = parse_series_entry(it);
        add_to_hash(idx, entry);
        rocksdb_iter_prev(it);
    }
    return 0;
}

// 查询时按需加载
series_index_t* index_get_or_load(index_hash_t* idx, uint64_t series_id) {
    series_index_t* entry = index_find(idx, series_id);
    if (!entry) {
        // 从持久化索引加载
        entry = load_series_from_disk(series_id);
        if (entry) {
            index_insert(idx, entry);
        }
    }
    return entry;
}
```

---

## 四、缓存层优化

### 4.1 多级缓存架构

```
┌─────────────────────────────────────────────┐
│               查询结果缓存                     │
│         (LRU, 大容量, 持久化)                 │
├─────────────────────────────────────────────┤
│               块数据缓存                       │
│        (LRU, 中容量, 内存)                    │
├─────────────────────────────────────────────┤
│               元数据缓存                       │
│       (Hash, 小容量, 热点数据)                │
└─────────────────────────────────────────────┘
```

### 4.2 块缓存实现

```c
typedef struct {
    uint64_t block_key;       // 块标识
    time_block_t* block;      // 块数据
    
    // LRU链表
    struct cache_entry* prev;
    struct cache_entry* next;
    
    // 统计
    uint32_t hit_count;
    uint64_t last_access;
} cache_entry_t;

typedef struct {
    cache_entry_t** entries;
    size_t capacity;
    size_t size;
    
    // LRU头尾
    cache_entry_t* lru_head;
    cache_entry_t* lru_tail;
    
    pthread_mutex_t lock;
} block_cache_t;

// 缓存查找
time_block_t* cache_get(block_cache_t* cache, uint64_t block_key) {
    cache_entry_t* entry = cache->entries[hash(block_key) % cache->capacity];
    while (entry) {
        if (entry->block_key == block_key) {
            // 移到LRU头部
            move_to_head(cache, entry);
            entry->hit_count++;
            return entry->block;
        }
        entry = entry->next;
    }
    return NULL;
}

// 缓存淘汰
void cache_evict(block_cache_t* cache) {
    // 淘汰尾部（最久未使用）
    cache_entry_t* victim = cache->lru_tail;
    remove_entry(cache, victim);
    
    // 写回磁盘（如果脏）
    if (victim->block->dirty) {
        flush_block_to_disk(victim->block);
    }
    free(victim->block);
    free(victim);
}
```

---

## 五、查询层优化

### 5.1 范围查询剪枝

```c
// 利用时间范围元数据跳过不相关数据块
int query_with_pruning(series_index_t* series, range_t* range, result_t* result) {
    // 快速判断：如果series时间范围与查询范围完全不重叠，直接跳过
    if (series->max_ts < range->start || series->min_ts > range->end) {
        return 0;
    }
    
    // 遍历所有块
    for (uint32_t i = 0; i < series->block_count; i++) {
        time_block_t* blk = &series->blocks[i];
        
        // 块级剪枝：计算块的时间范围
        uint64_t blk_start = blk->block_start;
        uint64_t blk_end = blk_start + blk->block_size;
        
        // 如果块与查询范围不重叠，跳过
        if (blk_end < range->start || blk_start > range->end) {
            continue;
        }
        
        // 块内扫描
        for (uint32_t j = 0; j < blk->count; j++) {
            uint64_t ts = blk_start + blk->timestamps[j];
            if (ts >= range->start && ts <= range->end) {
                add_result_point(result, ts, blk->values[j]);
            }
        }
    }
    return 0;
}
```

### 5.2 并行查询

```c
// 多线程并行扫描不同块
typedef struct {
    series_index_t* series;
    range_t range;
    result_t* result;
    uint32_t thread_id;
} query_task_t;

void* parallel_query_worker(void* arg) {
    query_task_t* task = (query_task_t*)arg;
    series_index_t* series = task->series;
    
    // 分块：每个线程处理一部分块
    uint32_t start_block = (series->block_count * task->thread_id) / NUM_THREADS;
    uint32_t end_block = (series->block_count * (task->thread_id + 1)) / NUM_THREADS;
    
    for (uint32_t i = start_block; i < end_block; i++) {
        scan_block(&series->blocks[i], &task->range, task->result);
    }
    return NULL;
}

int parallel_query(series_index_t* series, range_t* range, result_t* result) {
    pthread_t threads[NUM_THREADS];
    query_task_t tasks[NUM_THREADS];
    
    for (int i = 0; i < NUM_THREADS; i++) {
        tasks[i] = (query_task_t){series, *range, result, i};
        pthread_create(&threads[i], NULL, parallel_query_worker, &tasks[i]);
    }
    
    for (int i = 0; i < NUM_THREADS; i++) {
        pthread_join(threads[i], NULL);
    }
    return 0;
}
```

---

## 六、写入优化

### 6.1 批量写入与合并

```c
// 写入缓冲队列
typedef struct {
    point_t* points;
    size_t count;
    size_t capacity;
    
    // 异步刷新
    pthread_mutex_t lock;
    pthread_cond_t cond;
    bool shutdown;
} write_buffer_t;

// 批量刷盘
int flush_write_buffer(write_buffer_t* buf, rocksdb_t* db) {
    rocksdb_writebatch_t* batch = rocksdb_writebatch_create();
    
    pthread_mutex_lock(&buf->lock);
    for (size_t i = 0; i < buf->count; i++) {
        point_t* p = &buf->points[i];
        char key[64];
        make_key(key, sizeof(key), p);
        
        char value[32];
        snprintf(value, sizeof(value), "%.10f", p->value);
        
        rocksdb_writebatch_put(batch, key, strlen(key), value, strlen(value));
    }
    size_t count = buf->count;
    buf->count = 0;
    pthread_mutex_unlock(&buf->lock);
    
    rocksdb_write(db, NULL, batch, NULL);
    rocksdb_writebatch_destroy(batch);
    
    return count;
}
```

---

## 七、压缩优化

### 7.1 块内压缩

```c
// 简单Delta压缩
int compress_delta(uint32_t* src, uint8_t* dst, size_t src_len, size_t* dst_len) {
    if (src_len < 2) {
        memcpy(dst, src, src_len);
        *dst_len = src_len;
        return 0;
    }
    
    uint32_t prev = src[0];
    size_t j = 0;
    
    dst[j++] = (uint8_t)(prev & 0xFF);
    dst[j++] = (uint8_t)((prev >> 8) & 0xFF);
    dst[j++] = (uint8_t)((prev >> 16) & 0xFF);
    dst[j++] = (uint8_t)((prev >> 24) & 0xFF);
    
    for (size_t i = 1; i < src_len; i++) {
        int32_t delta = (int32_t)src[i] - (int32_t)prev;
        
        if (delta >= -128 && delta < 128) {
            dst[j++] = (uint8_t)(delta & 0xFF);
        } else {
            dst[j++] = 0x80;  // 标记：非压缩
            dst[j++] = (uint8_t)(delta & 0xFF);
            dst[j++] = (uint8_t)((delta >> 8) & 0xFF);
            dst[j++] = (uint8_t)((delta >> 16) & 0xFF);
            dst[j++] = (uint8_t)((delta >> 24) & 0xFF);
        }
        prev = src[i];
    }
    
    *dst_len = j;
    return 0;
}
```

### 7.2 RocksDB级别压缩配置

```c
rocksdb_options_t* create_optimized_options(void) {
    rocksdb_options_t* opts = rocksdb_options_create();
    
    // 基本配置
    rocksdb_options_set_create_if_missing(opts, 1);
    rocksdb_options_set_max_open_files(opts, 10000);
    
    // 写入缓冲
    rocksdb_options_set_write_buffer_size(opts, 64 * 1024 * 1024);  // 64MB
    rocksdb_options_set_max_write_buffer_number(opts, 4);
    
    // Level配置
    rocksdb_options_set_max_bytes_for_level_base(opts, 256 * 1024 * 1024);
    rocksdb_options_set_max_bytes_for_level_multiplier(opts, 10);
    
    // 压缩配置
    rocksdb_options_set_compression(opts, rocksdb_lz4_compression);
    rocksdb_options_set_compression_opts(opts, 5, 0, 0);
    
    // 布隆过滤器
    rocksdb_options_set_bloom_filter(opts, 10);
    
    // 缓存配置
    rocksdb_cache_t* cache = rocksdb_cache_create_lru(256 * 1024 * 1024);
    rocksdb_options_set_cache(opts, cache);
    
    return opts;
}
```

---

## 八、业务功能

### 8.1 多业务隔离

```c
// 每个业务独立DB实例
typedef struct {
    char business_name[64];
    rocksdb_t* db;
    tsdb_config_t config;
    
    // 保留策略
    int retention_days;
    bool enable_compression;
} business_instance_t;

// 初始化多业务
int init_multi_business(business_instance_t** instances, const char** names, size_t count) {
    for (size_t i = 0; i < count; i++) {
        char db_path[256];
        snprintf(db_path, sizeof(db_path), "./data/%s", names[i]);
        
        instances[i] = tsdb_open(db_path, NULL);
        if (!instances[i]) {
            return -1;
        }
    }
    return 0;
}
```

### 8.2 轻度汇总

```c
// 汇总任务
typedef struct {
    char measurement[128];
    char agg_type[16];      // "hourly", "daily", "monthly"
    uint64_t start_ts;
    uint64_t end_ts;
} agg_task_t;

// 异步汇总队列
typedef struct {
    agg_task_t* tasks;
    size_t count;
    size_t capacity;
    
    pthread_mutex_t lock;
    pthread_cond_t cond;
    pthread_t worker;
    
    // 汇总结果存储
    rocksdb_t* agg_db;
} agg_queue_t;

// 汇总Worker
void* agg_worker(void* arg) {
    agg_queue_t* queue = (agg_queue_t*)arg;
    
    while (!queue->shutdown) {
        agg_task_t task;
        
        pthread_mutex_lock(&queue->lock);
        while (queue->count == 0 && !queue->shutdown) {
            pthread_cond_wait(&queue->cond, &queue->lock);
        }
        
        if (queue->shutdown) break;
        
        task = queue->tasks[--queue->count];
        pthread_mutex_unlock(&queue->lock);
        
        // 执行汇总
        perform_aggregation(queue->agg_db, &task);
    }
    return NULL;
}
```

---

## 九、性能目标

| 指标 | 当前 | 优化目标 | 优化手段 |
|------|------|----------|----------|
| 写入吞吐 | 50K/s | 200K/s | 批量写入 + 块合并 |
| 查询延迟 | 50ms | 10ms | 范围剪枝 + 并行查询 |
| 内存效率 | 1M点/GB | 5M点/GB | 块压缩 + Delta编码 |
| 启动速度 | 1s | 0.1s | 延迟加载索引 |

---

## 十、实施计划

### Phase 1: 块存储（1周）
- [ ] 实现时间块数据结构
- [ ] 块写入/读取接口
- [ ] 块刷盘机制

### Phase 2: 缓存优化（1周）
- [ ] LRU块缓存
- [ ] 多级缓存架构
- [ ] 缓存淘汰策略

### Phase 3: 查询优化（1周）
- [ ] 范围剪枝
- [ ] 并行查询
- [ ] 向量化聚合

### Phase 4: 压缩优化（1周）
- [ ] Delta编码
- [ ] RocksDB压缩配置
- [ ] 压缩率测试

### Phase 5: 业务功能（1周）
- [ ] 多业务隔离
- [ ] 轻度汇总
- [ ] 保留策略
