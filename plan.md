# TSDB 实施计划 — 基于 idea.md 需求评审

> **评审日期**: 2026-04-15
> **当前版本**: 12 crates, ~60 tests, 0 编译错误
> **代码注释**: 全部源文件已添加中文文档注释

---

## 一、需求 vs 实现现状对照表

### 1.1 已完成功能 ✅

| # | idea.md 需求 | 实现位置 | 状态 |
|---|-------------|---------|------|
| 1 | Rust 开发 | 全项目 | ✅ |
| 2 | config.ini 可配置 | [config.rs](crates/tsdb-config/src/config.rs) | ✅ |
| 3 | NNG 服务接口 | [nng_transport.rs](crates/tsdb-server/src/nng_transport.rs) | ✅ REP+PULL+PUB 三模式 |
| 4 | GitHub 多平台发布 | [.github/workflows/ci.yml](.github/workflows/ci.yml) | ✅ Linux/Mac/Win × x86/aarch64 |
| 5 | TSBS 真实数据测试 | [main.rs load-tsbs](crates/tsdb-cli/src/main.rs) | ✅ DevOps 4000 设备生成 |
| 6 | 时间索引 SkipList | [skiplist.rs](crates/tsdb-index/src/skiplist.ts) | ✅ 范围查询 |
| 7 | 标签索引倒排索引 | [inverted.rs](crates/tsdb-index/src/inverted.rs) | ✅ RoaringBitmap |
| 8 | Delta 时间戳压缩 | [delta.rs](crates/tsdb-compress/src/delta.rs) | ✅ ZigZag+Varint |
| 9 | Gorilla XOR 浮点压缩 | [gorilla.rs](crates/tsdb-compress/src/gorilla.rs) | ✅ |
| 10 | 字典编码字符串压缩 | [dictionary.rs](crates/tsdb-compress/src/dictionary.rs) | ✅ |
| 11 | 向量化+SIMD聚合 | [simd_agg.rs](crates/tsdb-query/src/vectorized/simd_agg.rs) | ✅ Sum/Avg/Min/Max/Count |
| 12 | RocksDB 存储引擎 | [engine.rs](crates/tsdb-core/src/storage/engine.rs) | ✅ |
| 13 | MergeOperator 定制 | [merge_operator.rs](crates/tsdb-core/src/storage/merge_operator.rs) | ✅ MergedBlock upsert |
| 14 | RowKey+Qualifier 机制 | [rowkey.rs](crates/tsdb-core/src/rowkey.rs) | ✅ 30秒块+微秒偏移 |
| 15 | 冷热数据分离 | [cf_manager.rs](crates/tsdb-core/src/storage/cf_manager.rs) | ✅ 按日分CF |
| 16 | 插件机制(业务/查询/存储) | [traits.rs](crates/tsdb-plugin/src/traits.rs) | ✅ trait 定义 |
| 17 | 维度表字典编码 | [dimension.rs](crates/tsdb-core/src/storage/dimension.rs) | ✅ tag→ID 映射 |
| 18 | HTTP REST API | [http_api.rs](crates/tsdb-server/src/http_api.rs) | ✅ 7个端点 |
| 19 | 业务仪表盘HTML | [business.rs](crates/tsdb-dashboard/src/business.rs) | ✅ |
| 20 | 性能仪表盘HTML | [performance.rs](crates/tsdb-dashboard/src/performance.rs) | ✅ |
| 21 | SVG 图表渲染 | [svg.rs](crates/tsdb-chart/src/svg.rs) | ✅ Line/Area/Bar |
| 22 | SQL 解析器 | [parser.rs](crates/tsdb-query/src/parser.rs) | ✅ sqlparser crate |
| 23 | 查询规划器 | [plan.rs](crates/tsdb-query/src/plan.rs) | ✅ FullScan/IndexScan/Agg |
| 24 | BlockWriter 缓冲写入 | [block_writer.rs](crates/tsdb-core/src/storage/block_writer.rs) | ✅ |
| 25 | ExpoGo 移动前端 | [frontend/App.js](frontend/App.js) | ✅ 基础版 |

### 1.2 部分实现功能 🔄

| # | idea.md 需求 | 当前状态 | 差距 |
|---|-------------|---------|------|
| A | **多业务DB实例隔离** | server.rs 中仅有 `databases: HashMap` 且硬编码 `"default"` | 需支持按业务名创建独立 DB 实例 |
| B | **轻度汇总（时间维度）** | aggregator.rs 有 TimeDimension(hour/day/week/month) | 未接入写入路径，无独立存储 |
| C | **向量化执行引擎** | vectorized/ 目录有 ColumnarBatch+SimdAggregator | QueryEngine 未调用 VectorizedEngine |
| D | **ExpoGo 前端** | App.js 有 query/write/chart/status 四Tab | 缺少仪表盘页面、趋势图页面 |

### 1.3 未实现功能 ❌

| # | idea.md 需求 | 说明 | 优先级 |
|---|-------------|------|--------|
| 1 | **多业务隔离** | 股票行情、IOT、金融等不同业务复制到不同 DB 实例 | P0 |
| 2 | **轻度汇总异步管道** | 明细写入时触发异步聚合计算 | P0 |
| 3 | **轻度汇总独立存储** | 每种业务一个汇总DB，每个维度一个CF，长期保存 | P0 |
| 4 | **前缀分隔符压缩** | 轻度汇总 key 采用业务前缀+分隔符策略 | P1 |
| 5 | **前端仪表盘集成** | 在 ExpoGo 中嵌入 business/performance dashboard | P1 |
| 6 | **复杂SQL/JOIN** | 多表关联查询支持 | P2 |
| 7 | **前缀分隔符key压缩** | 汇总数据的 key 压缩策略 | P1 |

---

## 二、实施计划（分阶段）

### Phase 1: 核心缺失功能补全（P0 — 必须完成）

#### 任务 1.1: 多业务数据库隔离
- **目标**: 支持创建多个独立 DB 实例（如 `stocks`, `iot`, `finance`），互不干扰
- **改动文件**:
  - `tsdb-server/src/server.rs`: 扩展 `process_request()` 支持 `CreateDatabase/DropDatabase`
  - `tsdb-cli/src/main.rs`: 扩展 CLI 子命令支持 `--database` 参数
  - 新增 `tsdb-core/src/storage/multi_db.rs`: MultiDbManager 统一管理多个 StorageEngine
- **验收标准**: 可通过 API 创建/切换不同业务的数据库实例

#### 任务 1.2: 轻度汇总异步计算管道
- **目标**: 数据写入时自动触发异步聚合，按 hour/day/week/month 维度预计算
- **改动文件**:
  - `tsdb-aggregate/src/worker.rs`: 接入 BlockWriter 的 flush 回调
  - `tsdb-core/src/storage/engine.rs`: 写入成功后发送 NNG 消息到 Worker
  - 新增 `tsdb-aggregate/src/pipeline.rs`: LightAggregationPipeline 协调写入→聚合流程
- **验收标准**: 写入 10000 条 cpu 数据后，可查询到 hour/day 维度的聚合结果

#### 任务 1.3: 轻度汇总独立存储
- **目标**: 聚合结果存入独立的 RocksDB 实例（与明细分离），每种业务一个 DB，每个维度一个 CF
- **改动文件**:
  - 新增 `tsdb-aggregate/src/store.rs`: AggregationStore 管理聚合数据持久化
  - `tsdb-aggregate/src/aggregator.rs`: finalize() 结果写入 AggregationStore
  - `tsdb-config/src/config.rs`: 新增 `[aggregation]` 存储路径配置
- **验收标准**: 重启服务后聚合数据不丢失，按维度 CF 分离存储

### Phase 2: 查询增强（P1 — 重要优化）

#### 任务 2.1: 向量化引擎接入查询路径
- **目标**: QueryEngine 的聚合查询走 VectorizedEngine 路径
- **改动文件**:
  - `tsdb-query/src/engine.rs`: execute_aggregation() 内部委托给 VectorizedEngine
  - `tsdb-query/src/vectorized/vectorized.rs`: 增加 read_range → ColumnarBatch 转换
- **验收标准**: `SELECT AVG(usage) FROM cpu GROUP BY host` 使用 SIMD 加速

#### 任务 2.2: 前缀分隔符 Key 压缩
- **目标**: 轻度汇总数据采用 `business|dimension|timestamp` 格式的 key 压缩
- **改动文件**:
  - 新增 `tsdb-aggregate/src/key_codec.rs`: 汇总 key 的编解码
  - `tsdb-aggregate/src/store.rs`: 使用 key_codec 存取数据
- **验收标准**: 相同业务+维度的 key 共享前缀字节

### Phase 3: 前端增强（P1 — 用户体验）

#### 任务 3.1: ExpoGo 仪表盘页面
- **目标**: 在移动端展示业务仪表盘和性能仪表盘
- **改动文件**:
  - `frontend/App.js`: 新增 Dashboard Tab，内嵌 WebView 或原生组件
  - 新增 `frontend/screens/DashboardScreen.js`: 业务指标卡片网格
  - 新增 `frontend/screens/PerformanceScreen.js`: 进度条+系统概览
- **验收标准**: 移动端可查看 CPU/内存等指标的实时仪表盘

#### 任务 3.2: 时序图增强
- **目标**: 前端支持多序列对比图、缩放、时间范围选择
- **改动文件**:
  - `frontend/App.js`: Chart Tab 增强，支持 victory-native 图表库
  - 新增 `frontend/components/TimeseriesChart.js`: 多线图组件
- **验收标准**: 可在手机上查看多指标叠加的折线图

### Phase 4: 高级特性（P2 — 未来迭代）

#### 任务 4.1: 复杂 SQL / JOIN
- **目标**: 支持跨 measurement 关联查询
- **预估工作量**: 较大，需扩展 sqlparser 和执行引擎

#### 任务 4.2: C++ RocksDB 插件定制
- **目标**: 用 C++ 编写自定义 Comparator/CompactionFilter
- **预估工作量**: 需要 rocksdb-sys FFI 绑定

---

## 三、架构改进建议

### 3.1 当前架构图（已实现部分）

```
┌──────────────────────────────────────────────────────┐
│                    TSDB Server                        │
│  ┌──────────┐ ┌──────────┐ ┌─────────────────────┐  │
│  │ TCP Server│ │HTTP API  │ │NNG REP/PULL/PUB     │  │
│  │ :7878     │ │ :7879    │ │ :9911/:9912/:9913   │  │
│  └────┬─────┘ └────┬─────┘ └────────┬────────────┘  │
│       └────────────┼────────────────┘               │
│                    ▼                                  │
│  ┌─────────────────────────────────────────────┐   │
│  │           TsdbServer (单 DB "default")        │   │
│  │  process_request() → StorageEngine            │   │
│  └─────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────┘
```

### 3.2 目标架构图（Phase 1 完成后）

```
┌──────────────────────────────────────────────────────────┐
│                      TSDB Server                         │
│                                                          │
│  ┌────────┐  ┌────────┐  ┌────────┐  ┌────────────────┐ │
│  │ TCP    │  │ HTTP   │  │ NNG    │  │ ExpoGo Frontend│ │
│  │ :7878  │  │ :7879  │  │ 3端口  │  │ Mobile/Desktop │ │
│  └───┬────┘  └───┬────┘  └───┬────┘  └───────┬────────┘ │
│      └─────────────┼───────────┘                │          │
│                    ▼                             │          │
│  ┌─────────────────────────────────────────────────┴──┐ │
│  │              MultiDbManager                       │ │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐             │ │
│  │  │ stocks  │ │ iot     │ │ finance │ ...         │ │
│  │  │ (DB实例)│ │ (DB实例)│ │ (DB实例)│             │ │
│  │  └────┬────┘ └────┬────┘ └────┬────┘             │ │
│  └───────┼──────────┼──────────┼──────────────────────┘ │
│          ▼          ▼          ▼                     │
│  ┌──────────────────────────────────────────────────┐  │
│  │         LightAggregationPipeline (异步)          │  │
│  │  写入触发 → NNG → Worker → 按维度聚合 → 汇总DB  │  │
│  │  [hour] [day] [week] [month] 各自独立 CF        │  │
│  └──────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────┘
```

---

## 四、实施优先级排序

| 优先级 | 任务 | 预估工作量 | 依赖关系 |
|--------|------|-----------|---------|
| **P0-1** | 1.1 多业务DB隔离 | 中 | 无 |
| **P0-2** | 1.2 异步聚合管道 | 中 | 依赖 1.1 |
| **P0-3** | 1.3 聚合独立存储 | 中 | 依赖 1.2 |
| **P1-1** | 2.1 向量化引擎接入 | 小 | 无 |
| **P1-2** | 2.2 前缀Key压缩 | 小 | 依赖 1.3 |
| **P1-3** | 3.1 ExpoGo仪表盘 | 中 | 无 |
| **P1-4** | 3.2 时序图增强 | 中 | 依赖 3.1 |
| **P2-1** | 4.1 复杂SQL/JOIN | 大 | 依赖全部P1 |
| **P2-2** | 4.2 C++插件定制 | 大 | 独立任务 |

---

## 五、风险与注意事项

1. **RocksDB 多实例开销**: 每个 DB 实例占用独立内存和文件句柄，需要控制最大实例数
2. **聚合延迟**: 异步聚合有秒级延迟，对实时性要求高的场景需提供"强制刷新"API
3. **前端兼容性**: ExpoGo 的 react-native-svg 在 Android 上可能有性能问题，需测试
4. **CI 资源**: Windows 交叉编译 aarch64 可能需要额外工具链配置
