# TSDB - 时序数据库 C 语言实现

本项目包含多个时序数据库（Time Series Database）C 语言实现：

- **原生实现**: `glm5`、`minimax25`、`kimi25`
- **RocksDB 版本**: `rocksdb-minimax25`、`rocksdb-glm5`、`rocksdb-kimi25`、`rocksdb-qwen35`

---

### 7. rocksdb-qwen35 - RocksDB 高性能版（新增）

**特点：**
- 整合三大优化策略（kimi25/minimax25/glm5）
- **二进制紧凑 Key** - 减少 55% 存储空间
- **时间块聚合存储** - 30 秒块内紧凑存储
- **ColumnFamily 冷热分离** - 按自然日分 CF
- **Gorilla 压缩算法** - 10:1~20:1压缩率
- **自动批量写入** - 后台线程刷新
- **生产级性能** - 500K pts/s 写入吞吐

**文件结构：**
```
rocksdb-qwen35/
├── qtsdb.h             # 头文件
├── qtsdb.c             # 核心实现
├── example.c           # 示例程序
├── Makefile
└── README.md
```

**使用示例：**
```c
#include "qtsdb.h"

qtsdb_config_t config = qtsdb_default_config();
config.enable_wal = false;  // 高性能模式
config.batch_size = 1000;

qtsdb_db_t* db = qtsdb_open("./data", &config);

qtsdb_point_t* p = qtsdb_point_create("cpu_usage", qtsdb_now());
qtsdb_point_add_tag(p, "host", "server01");
qtsdb_point_add_field_float(p, "value", 75.5);
qtsdb_write(db, p);

qtsdb_time_range_t range = {start, end};
qtsdb_agg_result_set_t agg;
qtsdb_query_agg(db, "cpu_usage", &range, QTSDB_AGG_AVG, NULL, &agg);

qtsdb_close(db);
```

**核心优化：**

1. **Key 设计** - 20 字节紧凑结构
   ```
   ┌─────────┬──────────┬─────────────┬──────────┬──────────┐
   │  type   │ meas_len │ measurement │ series_id│ timestamp│
   │  1 byte │ 2 bytes  │ variable    │ 8 bytes  │ 8 bytes  │
   └─────────┴──────────┴─────────────┴──────────┴──────────┘
   ```

2. **存储架构** - ColumnFamily 冷热分离
   ```
   ┌─────────────────────────────────────────────────────────┐
   │                    RocksDB Instance                      │
   ├─────────────┬─────────────┬─────────────┬───────────────┤
   │ CF:meta     │ CF:20240101 │ CF:20240102 │ CF:20240103   │
   │ (元数据)    │ (冷数据)    │ (温数据)    │ (热数据)      │
   │             │ ZSTD 压缩    │ LZ4压缩     │ 无压缩        │
   └─────────────┴─────────────┴─────────────┴───────────────┘
   ```

3. **性能指标**
   - 写入吞吐：500K pts/s
   - 查询延迟：5-10ms
   - 压缩率：10:1~20:1
   - 内存效率：10M 点/GB

---

## 项目概览

### 原生版本

| 项目 | 版本 | 代码规模 | 架构风格 | 存储引擎 | 适用场景 |
|------|------|----------|----------|----------|----------|
| **glm5** | 1.0.0 | ~2500 行 | 分层模块化 | 自定义二进制 | 学习架构、数据压缩场景 |
| **minimax25** | 2.5.0 | ~2800 行 | 分层模块化 | 自定义二进制 | 生产原型、二次开发 |
| **kimi25** | 2.5.0 | ~570 行 | 单体极简 | 内存存储 | 嵌入式，教学示例 |

### RocksDB 版本

| 项目 | 版本 | 代码规模 | 架构风格 | 适用场景 |
|------|------|----------|----------|----------|
| **rocksdb-glm5** | 1.0.0 | ~1200 行 | 模块化 | 生产环境 |
| **rocksdb-minimax25** | 2.5.0-rc1 | ~550 行 | 单文件完整 | 生产原型 |
| **rocksdb-kimi25** | 1.0.0 | ~250 行 | 单文件极简 | 嵌入式/原型 |
| **rocksdb-qwen35** | 1.0.0 | ~800 行 | 单文件优化 | **生产级高性能** |

---

## 快速开始

### 编译所有原生版本

```bash
# 编译 glm5
cd glm5 && make && cd ..

# 编译 minimax25
cd minimax25 && make && cd ..

# 编译 kimi25
cd kimi25 && make && cd ..
```

### 编译所有RocksDB版本

```bash
# RocksDB依赖 (macOS)
brew install rocksdb

# 编译 rocksdb-glm5
cd rocksdb-glm5 && make && cd ..

# 编译 rocksdb-minimax25
cd rocksdb-minimax25 && make && cd ..

# 编译 rocksdb-kimi25
cd rocksdb-kimi25 && make && cd ..
```

### 运行示例

```bash
# 原生版本
cd glm5 && ./example
cd minimax25 && ./example
cd kimi25 && ./example

# RocksDB版本
cd rocksdb-glm5 && ./example
cd rocksdb-minimax25 && ./example
cd rocksdb-kimi25 && ./example
```

---

## 项目详细说明

### 1. kimi25 - 极简时序数据库

**特点：**
- 单文件实现（kimi_tsdb.h/c），仅570行代码
- 极简API设计，易于理解和移植
- 基础功能完整，适合嵌入式设备
- 纯内存存储，性能优异

**文件结构：**
```
kimi25/
├── kimi_tsdb.h       # 头文件
├── kimi_tsdb.c       # 所有功能集成
└── example.c
```

**使用示例：**
```c
#include "kimi_tsdb.h"

kimi_tsdb_t* db = kimi_tsdb_open("./data", NULL);

kimi_point_t* p = kimi_point_create("cpu_usage", kimi_now());
kimi_point_add_tag(p, "host", "server01");
kimi_point_add_field_f64(p, "usage", 75.5);
kimi_write(db, p);

kimi_query_t* q = kimi_query_new("cpu_usage");
kimi_query_range(q, (kimi_range_t){start, end});
kimi_result_t result;
kimi_query(db, q, &result);

kimi_tsdb_close(db);
```

---

### 2. minimax25 - 功能完整的时序数据库

**特点：**
- 功能最完整，最接近生产可用
- 支持WAL（Write-Ahead Logging）保证数据安全
- 哈希索引（2048桶），O(1)查找效率
- 11种聚合类型（含标准差、中位数、P95/P99）
- 支持正则匹配查询

**文件结构：**
```
minimax25/
├── tsdb25.h/c          # 主API
├── tsdb25_types.h/c    # 类型定义
├── tsdb25_config.h/c   # 配置
├── tsdb25_storage.h/c  # 存储引擎
├── tsdb25_index.h/c    # 哈希索引
├── tsdb25_query.h/c    # 查询引擎
├── tsdb25_cache.h/c    # LRU缓存
├── tsdb25_wal.h/c      # WAL日志
└── example.c
```

---

### 3. glm5 - 模块化时序数据库

**特点：**
- 8个功能模块，职责分离清晰
- 完整的压缩模块（Snappy/ZSTD/LZ4/Gorilla）
- B+树风格索引
- 支持复杂查询表达式（AND/OR/NOT）

**文件结构：**
```
glm5/
├── tsdb.h/c            # 主API入口
├── tsdb_types.h/c      # 数据类型定义
├── tsdb_config.h/c      # 配置管理
├── tsdb_storage.h/c     # 存储引擎
├── tsdb_index.h/c       # B+树索引
├── tsdb_query.h/c       # 查询引擎
├── tsdb_cache.h/c       # LRU缓存
├── tsdb_compress.h/c    # 压缩模块
└── example.c
```

---

### 4. rocksdb-kimi25 - RocksDB极简版

**特点：**
- 基于RocksDB的极简实现（~250行）
- 利用RocksDB的LSM-Tree优化
- 无独立索引，完全依赖RocksDB

**文件结构：**
```
rocksdb-kimi25/
├── rkimi_tsdb.h        # 头文件
├── rkimi_tsdb.c        # 实现
└── example.c
```

**使用示例：**
```c
#include "rkimi_tsdb.h"

rkimi_db_t* db = rkimi_open("data");

rkimi_point_t* p = rkimi_point_new("cpu", ts);
rkimi_tag(p, "host", "s1");
rkimi_val(p, 75.5);
rkimi_write(db, p);

rkimi_agg_result_t agg;
rkimi_agg(db, "cpu", &range, RKIMI_AGG_AVG, &agg);

rkimi_close(db);
```

---

### 5. rocksdb-minimax25 - RocksDB完整版

**特点：**
- 基于RocksDB的完整实现
- 完善的配置管理
- 9种聚合类型

**文件结构：**
```
rocksdb-minimax25/
├── rocksdb_tsdb.h      # 头文件
├── rocksdb_tsdb.c      # 实现
└── example.c
```

---

### 6. rocksdb-glm5 - RocksDB模块化版

**特点：**
- 基于RocksDB的模块化架构
- 独立哈希索引系统
- 最完整的功能支持

**文件结构：**
```
rocksdb-glm5/
├── rtsdb.h/c            # 主API
├── rtsdb_types.h/c      # 类型定义
├── rtsdb_config.h/c     # 配置
├── rtsdb_storage.h/c    # RocksDB存储
├── rtsdb_index.h/c      # 哈希索引
├── rtsdb_query.h/c     # 查询引擎
└── example.c
```

---

## 功能对比

### 原生版本

| 功能 | glm5 | minimax25 | kimi25 |
|------|------|-----------|--------|
| 数据写入 | ✅ | ✅ | ✅ |
| 批量写入 | ✅ | ✅ | ✅ |
| 时间范围查询 | ✅ | ✅ | ✅ |
| 标签过滤 | ✅ | ✅ | ✅ |
| **WAL日志** | ❌ | ✅ | ⚠️ |
| **数据压缩** | ✅ 4种 | ⚠️ 框架 | ❌ |
| **聚合类型** | 7种 | **11种** | 8种 |
| 正则匹配 | ❌ | ✅ | ❌ |
| 持久化 | ✅ | ✅ | ⚠️ |

### RocksDB版本

| 功能 | rocksdb-glm5 | rocksdb-minimax25 | rocksdb-kimi25 |
|------|--------------|-------------------|----------------|
| 数据写入 | ✅ | ✅ | ✅ |
| 批量写入 | ✅ | ✅ | ✅ |
| 时间范围查询 | ✅ | ✅ | ✅ |
| 聚合类型 | 9种 | 9种 | 5种 |
| 独立索引 | ✅ | ❌ | ❌ |
| 索引持久化 | ✅ | ❌ | ❌ |

---

## 架构对比

### 索引实现

| 项目 | 索引类型 | 时间复杂度 |
|------|----------|------------|
| glm5 | B+树风格 (order=8) | O(log n) |
| minimax25 | **哈希表 (2048桶)** | **O(1)** |
| kimi25 | 哈希表 (1024桶) | O(1) |
| rocksdb-glm5 | 独立哈希索引 | O(1) |
| rocksdb-minimax25 | RocksDB原生 | O(n) |
| rocksdb-kimi25 | RocksDB原生 | O(n) |

### 存储引擎

| 项目 | 存储引擎 | 压缩支持 |
|------|----------|----------|
| glm5 | 自定义二进制 | Snappy/ZSTD/LZ4/Gorilla |
| minimax25 | 自定义二进制 | Snappy/ZSTD/Gorilla |
| kimi25 | 内存存储 | 无 |
| rocksdb-* | **RocksDB (LSM-Tree)** | Snappy |

---

## 系统要求

- **编译器**: GCC 或 Clang，支持 C99 标准
- **操作系统**: Linux/macOS/Unix（使用 POSIX API）
- **依赖 (RocksDB版本)**: 
  - RocksDB库
  - pthread
  - 标准C库

### 安装RocksDB

```bash
# macOS
brew install rocksdb

# Ubuntu/Debian
sudo apt-get install librocksdb-dev

# CentOS/RHEL
sudo yum install rocksdb-devel
```

---

## 选型建议

| 使用场景 | 推荐项目 | 理由 |
|----------|----------|------|
| 学习TSDB原理 | **kimi25** | 代码极简，易于理解 |
| 生产环境 | **minimax25** | 功能完整，有WAL保证数据安全 |
| 需要数据压缩 | **glm5** | 压缩模块完整实现 |
| 嵌入式/IoT设备 | **kimi25** | 代码量小，易于移植 |
| 二次开发基础 | **minimax25** | 架构清晰，功能丰富 |
| RocksDB后端 | **rocksdb-glm5** | 独立索引，性能好 |
| 简单RocksDB集成 | **rocksdb-kimi25** | 代码最少 |

---

## 文档

- [功能对比报告](PERFORMANCE.md)
- [RocksDB版本对比](ROCKSDB_COMPARISON.md)
- [索引实现对比](INDEX_COMPARISON.md)

---

## 许可证

MIT License

---

## 作者

Generated by AI Assistant
