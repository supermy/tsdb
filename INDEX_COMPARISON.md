# RocksDB时序数据库索引实现对比

## 索引架构总览

| 版本 | 索引类型 | 实现方式 | 复杂度 |
|------|----------|----------|--------|
| **rocksdb-kimi25** | 无独立索引 | RocksDB原生 | 极简 |
| **rocksdb-minimax25** | 无独立索引 | RocksDB原生 | 简单 |
| **rocksdb-glm5** | 哈希索引 | 独立实现 | 完整 |

---

## 1. rocksdb-glm5 - 完整哈希索引

### 索引结构

```c
#define HASH_BUCKET_COUNT 2048

typedef struct index_bucket {
    rtsdb_series_info_t* series;
    struct index_bucket* next;
} index_bucket_t;

struct rtsdb_index {
    index_bucket_t** buckets;           // 哈希桶数组
    size_t           bucket_count;       // 桶数量 (2048)
    size_t           series_count;       // 总series数
    size_t           measurement_count;  // measurement数
    
    rtsdb_series_info_t** all_series;   // 所有series数组
    size_t           all_series_cap;
    
    char** measurements;                // measurement列表
    size_t           measurements_cap;
};
```

### 核心算法

#### 哈希函数
```c
static size_t hash_series_id(uint64_t id, size_t bucket_count) {
    return id % bucket_count;  // 简单取模
}
```

#### series_id生成 (FNV-1a)
```c
static uint64_t fnv1a_hash64(const void* data, size_t len) {
    const uint8_t* bytes = (const uint8_t*)data;
    uint64_t hash = 14695981039346656037ULL;
    for (size_t i = 0; i < len; i++) {
        hash ^= bytes[i];
        hash *= 1099511628211ULL;
    }
    return hash;
}
```

### 索引操作

| 操作 | 时间复杂度 | 说明 |
|------|-----------|------|
| 插入 | O(1) | 哈希定位 + 链表头插 |
| 查找 | O(1) avg | 哈希定位 + 链表遍历 |
| 删除 | O(1) avg | 哈希定位 + 链表删除 |
| 按measurement查找 | O(n) | 遍历所有series |

### 数据组织

```
┌─────────────────────────────────────────────────────────┐
│                     Index (Memory)                      │
├─────────────────────────────────────────────────────────┤
│  Bucket[0]  →  series_1 → series_5 → ...             │
│  Bucket[1]  →  series_2 → series_8 → ...             │
│  Bucket[2]  →  series_3 → ...                        │
│  ...                                                    │
│  Bucket[2047]                                         │
├─────────────────────────────────────────────────────────┤
│  all_series[]  (线性数组，快速遍历)                     │
│  measurements[] (measurement列表)                       │
└─────────────────────────────────────────────────────────┘
```

### 索引持久化

```c
rtsdb_status_t rtsdb_index_persist(rtsdb_index_t* index, const char* path) {
    FILE* fp = fopen(path, "wb");
    fwrite(&index->series_count, sizeof(size_t), 1, fp);
    fwrite(&index->measurement_count, sizeof(size_t), 1, fp);
    
    for (size_t i = 0; i < index->series_count; i++) {
        fwrite(index->all_series[i], sizeof(rtsdb_series_info_t), 1, fp);
    }
    fclose(fp);
    return RTSDB_OK;
}
```

### 特点总结

✅ **优点**:
- O(1) 平均查找时间
- 支持按measurement快速筛选
- 独立索引，可持久化
- 完整的series元数据管理

⚠️ **缺点**:
- 需要额外内存
- 启动时需加载索引

---

## 2. rocksdb-minimax25 - RocksDB原生索引

### 架构特点

- **无独立索引结构**
- **完全依赖RocksDB的LSM-Tree**
- 数据Key格式: `{measurement}_{series_id}_{timestamp}`

### 存储结构

```c
// Key: cpu_usage_123456789_1700000000000000000
// Value: 75.5
```

### 查询流程

```c
rtsdb_status_t rocksdb_tsdb_query(...) {
    // 1. 构建前缀
    char prefix[256];
    snprintf(prefix, sizeof(prefix), "%s_", measurement);
    
    // 2. 使用RocksDB Iterator扫描
    rocksdb_iterator_t* iter = rocksdb_create_iterator(db, read_opts);
    rocksdb_iter_seek(iter, prefix, prefix_len);
    
    // 3. 遍历匹配
    while (rocksdb_iter_valid(iter)) {
        // 时间范围过滤
        if (timestamp >= range->start && timestamp <= range->end) {
            // 添加到结果
        }
        rocksdb_iter_next(iter);
    }
}
```

### 特点总结

✅ **优点**:
- 无额外索引内存开销
- RocksDB自动优化（Bloom Filter）
- 代码简洁

⚠️ **缺点**:
- 扫描整个measurement的所有数据
- 无法快速获取series列表

---

## 3. rocksdb-kimi25 - 极简设计

### 架构特点

- **无独立索引**
- **最简Key设计**
- 仅用于数据组织，无元数据管理

### 存储结构

```c
// Key: cpu_123456789_1700000000000000000
// Value: 75.5
```

### series_id计算（仅用于Key生成）

```c
static uint64_t hash_series(const rkimi_point_t* p) {
    uint64_t h = 14695981039346656037ULL;
    const uint8_t* d = (const uint8_t*)p->measurement;
    for (size_t i = 0; i < strlen(p->measurement); i++) {
        h ^= d[i]; h *= 1099511628211ULL;
    }
    // 标签也参与哈希
    for (size_t i = 0; i < p->tag_count; i++) {
        // ... 标签key/value哈希
    }
    return h;
}
```

### 特点总结

✅ **优点**:
- 代码最少（~250行）
- 无额外内存开销
- 完全依赖RocksDB

⚠️ **缺点**:
- 无任何索引元数据
- 无法获取series信息

---

## 索引性能对比

### 查询性能分析

| 查询类型 | rocksdb-glm5 | rocksdb-minimax25 | rocksdb-kimi25 |
|---------|--------------|-------------------|----------------|
| 按measurement全量 | O(n) 扫描 | O(n) 扫描 | O(n) 扫描 |
| 时间范围查询 | O(n) 扫描 | O(n) 扫描 | O(n) 扫描 |
| 获取series列表 | **O(1)** | O(n) 扫描 | ❌ 不支持 |
| 获取measurement列表 | **O(1)** | O(n) 扫描 | ❌ 不支持 |

### 内存使用

| 版本 | 索引内存开销 | 说明 |
|------|-------------|------|
| rocksdb-glm5 | ~10-50MB | 哈希表 + 元数据 |
| rocksdb-minimax25 | 0 | RocksDB内部 |
| rocksdb-kimi25 | 0 | 无额外索引 |

### 启动时间

| 版本 | 冷启动 | 热启动 |
|------|--------|--------|
| rocksdb-glm5 | 较慢 | 快 |
| rocksdb-minimax25 | 快 | 快 |
| rocksdb-kimi25 | 最快 | 快 |

---

## 总结对比表

| 特性 | rocksdb-glm5 | rocksdb-minimax25 | rocksdb-kimi25 |
|------|---------------|-------------------|----------------|
| **索引类型** | 哈希索引 | RocksDB原生 | RocksDB原生 |
| **哈希桶数** | 2048 | N/A | N/A |
| **series查找** | O(1) | O(n) | O(n) |
| **measurement查找** | O(n) | O(n) | O(n) |
| **元数据存储** | ✅ 完整 | ❌ | ❌ |
| **索引持久化** | ✅ | ❌ | ❌ |
| **内存开销** | 中等 | 低 | 极低 |
| **代码复杂度** | 高 | 低 | 极低 |

## 选型建议

| 场景 | 推荐 | 理由 |
|------|------|------|
| 需要series列表 | rocksdb-glm5 | 独立索引支持 |
| 需要元数据统计 | rocksdb-glm5 | 完整元数据 |
| 高吞吐量写入 | rocksdb-kimi25 | 最小开销 |
| 简单查询 | rocksdb-minimax25 | 平衡选择 |
| 内存受限 | rocksdb-kimi25 | 无额外内存 |
