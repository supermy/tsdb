# TSDB TDD 实施计划 — 从功能开发到上线

> **计划版本**: v2.0
> **基准状态**: 146 测试通过, 0 编译错误, 0 clippy 警告, CI 通过 (commit f6623ea)
> **核心理念**: 测试驱动开发 (TDD)，每个阶段使用不同业务场景的真实数据

---

## 阶段总览

```
Phase 1: 功能开发 + 单元测试          ← 当前位置
Phase 2: 业务联调 + 系统集成测试
Phase 3: 压力性能测试
Phase 4: 系统部署 (Docker/K8s)
Phase 5: 安全测试
Phase 6: 上线联调 + 灰度发布
Phase 7: 生产监控 + 运维
```

---

## Phase 1: 功能开发 + 单元测试

### 目标

补全所有缺失的核心功能，每个功能**先写测试，再写实现**。

### 业务数据集 A: IoT 设备监控

模拟 **智能工厂设备监控** 场景：
- measurement: `sensor_temp`, `sensor_humidity`, `motor_vibration`, `power_consumption`
- tags: `device_id` (D001-D100), `factory_line` (A/B/C), `location` (workshop/warehouse/office)
- fields: `value` (f64), `status` (string), `alarm_level` (i64)
- 数据特征: 每 10 秒一条，100 台设备，持续 24 小时

#### 1.1 缺失功能清单与测试优先级

| # | 功能 | 文件 | 测试数 | 业务数据 | 优先级 |
|---|------|------|--------|---------|--------|
| F1 | NNG 与 TsdbServer 集成 | server.rs | 5 | 启动/停止/多协议并发 | P0 |
| F2 | 异步 I/O 改造 (tokio) | server.rs, http_api.rs | 8 | async handler 并发写入 | P0 |
| F3 | HTTP API 完整 CRUD | http_api.rs | 12 | CreateDB/DropDB/ListDB/Ping | P0 |
| F4 | 查询引擎 FullScan 完善 | engine.rs (query) | 10 | WHERE 时间范围+标签过滤 | P0 |
| F5 | 查询引擎 IndexScan 实现 | engine.rs (query) | 8 | 利用 InvertedIndex 过滤 | P1 |
| F6 | BlockWriter flush 触发机制 | block_writer.rs | 4 | max_rows / time_based flush | P1 |
| F7 | CFManager 自动清理过期 CF | cf_manager.rs | 3 | retention_days 清理 | P1 |
| F8 | DimensionTable 持久化 | dimension.rs | 4 | encode/decode 往返 + 反向映射 | P1 |
| F9 | PluginRegistry 动态加载 | registry.rs | 3 | 注册/查找/验证 | P2 |
| F10 | TimeseriesGenerator 多维对比 | timeseries.rs | 5 | 同比/环比/趋势预测 | P2 |

#### 1.2 每个功能的 TDD 工作流

```
1. RED:   写失败测试（描述期望行为）
2. GREEN: 最简实现使测试通过
3. REFACTOR: 重构保持测试通过
4. REPEAT: 下一个测试用例
```

**示例 — F3 HTTP API CreateDatabase 测试**:
```rust
#[tokio::test]
async fn test_create_database_iot_scenario() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_mgr = MultiDbManager::new(dir.path().to_path_buf(), CfConfig::default());
    let api = HttpApi::new(db_mgr);

    // 创建 "iot_sensors" 数据库
    let resp = api.handle_create_database("iot_sensors").await;
    assert!(resp.is_ok());

    // 重复创建应失败
    let resp = api.handle_create_database("iot_sensors").await;
    assert!(resp.is_err());

    // 列出数据库应包含新库
    let dbs = api.handle_list_databases().await.unwrap();
    assert!(dbs.contains(&"iot_sensors".to_string()));
}
```

#### 1.3 验收标准

| 指标 | Phase 1 前 | Phase 1 后 |
|------|-----------|-----------|
| 单元测试数 | 146 | > 250 |
| 功能覆盖率 (核心模块) | ~60% | > 90% |
| clippy warnings | 0 | 0 |
| CI 通过率 | 100% | 100% |

---

## Phase 2: 业务联调 + 系统集成测试

### 目标

验证跨模块端到端数据流正确性。使用**不同业务数据集**。

### 业务数据集 B: 金融交易流水

模拟 **支付网关交易记录** 场景：
- measurement: `payment`, `refund`, `settlement`, `fraud_alert`
- tags: `merchant_id` (M0001-M0500), `channel` (alipay/wechat/card), `currency` (CNY/USD/EUR)
- fields: `amount` (f64), `fee` (f64), `status_code` (i64), `risk_score` (f64)
- 数据特征: 高频写入 (QPS 5000+), 严格 ACID 要求, 时间窗口聚合

#### 2.1 集成测试模块设计

新建 `crates/tsdb-integration-tests/src/` 下的测试文件：

| 测试文件 | 覆盖路径 | 测试数 | 描述 |
|---------|---------|--------|------|
| `write_read_pipeline.rs` | HTTP→Engine→RocksDB→read_range | 15 | 写入金融交易 → 按时间/标签查询 → 验证数据完整性 |
| `merge_write_path.rs` | write_merged→MergeOperator→MergedBlock | 8 | 高频小额交易合并写入 → 读取验证字段覆盖 |
| `compress_roundtrip.rs` | Engine→BlockCodec→Gorilla/Delta→解压 | 6 | 金额字段 XOR 压缩往返精度验证 (< 0.01 偏差) |
| `aggregation_e2e.rs` | Pipeline→Aggregator→Store→Timeseries | 10 | 按商户/渠道/币种分维度聚合 → 图表生成 |
| `index_e2e.rs` | IndexManager→InvertedIndex+WAL→查询 | 8 | 按渠道+币种索引 → 交集查询 → 验证结果 |
| `protocol_v2_e2e.rs` | TCP Client→Envelope→Server→Response | 6 | 二进制协议编解码 + CRC32 校验完整链路 |
| `multi_db_isolation.rs` | MultiDbManager→多实例隔离 | 5 | 不同业务 DB 数据完全隔离 |
| `concurrent_writes.rs` | 多线程并发写入同一 DB | 4 | 100 并发写入无数据丢失/损坏 |

#### 2.2 关键集成测试示例

```rust
// write_read_pipeline.rs — 金融交易端到端
#[test]
fn test_payment_write_and_query_by_time_range() {
    let dir = tempfile::TempDir::new().unwrap();
    let engine = StorageEngine::open(dir.path(), CfConfig::default()).unwrap();

    // 写入 1000 条支付记录 (不同商户、渠道、时间)
    for i in 0..1000 {
        let ts = 1700000000_000_000 + i * 1000; // 每秒 1 条
        let mut dp = DataPoint::new("payment", ts);
        dp.tags.insert("merchant_id".to_string(), format!("M{:04}", i % 50));
        dp.tags.insert("channel".to_string(), ["alipay", "wechat", "card"][i % 3].to_string());
        dp.fields.insert("amount".to_string(), FieldValue::Float(100.0 + (i as f64) * 0.01));
        dp.fields.insert("fee".to_string(), FieldValue::Float(0.6));
        engine.write(&dp).unwrap();
    }

    // 查询 alipay 渠道最近 500 条
    let results = engine.read_range(
        "payment",
        &Tags::new(),
        1700000000_000_000,
        1700005000_000_000,
    ).unwrap();

    // 验证: 结果非空, 字段完整, 时间有序
    assert!(!results.is_empty());
    for (i, dp) in results.iter().enumerate() {
        if i > 0 { assert!(dp.timestamp >= results[i-1].timestamp); }
        assert!(dp.fields.contains_key("amount"));
        assert!(dp.fields.contains_key("fee"));
    }
}
```

#### 2.3 验收标准

| 指标 | Phase 2 前 | Phase 2 后 |
|------|-----------|-----------|
| 集成测试数 | 16 (已有) | > 60 |
| 端到端路径覆盖率 | ~40% | > 85% |
| 多线程安全 | 未验证 | 全部通过 |
| 数据一致性 | 未验证 | 写入=读取 100% |

---

## Phase 3: 压力性能测试

### 目标

验证系统在高负载下的表现，定位瓶颈。

### 业务数据集 C: 日志/指标海量写入

模拟 **Kubernetes 集群监控** 场景：
- measurement: `container_metrics`, `pod_logs`, `node_stats`, `network_flow`
- tags: `cluster` (prod/staging/dev), `namespace` (default/kube-system), `pod_name`, `node_ip`
- fields: `cpu_usage` (f64), `memory_mb` (f64), `request_count` (i64), `latency_us` (i64)
- 数据特征: **10 万 QPS**, 5000+ pod, 每条 < 200 bytes, 持续 30 分钟

#### 3.1 性能测试矩阵

| 测试场景 | 并发数 | 数据量 | 指标 | 目标值 |
|---------|--------|--------|------|--------|
| 纯写入吞吐 | 1/10/50/100 线程 | 100 万条 | writes/sec | > 50K/sec (单线程), > 200K/sec (100线程) |
| 写入延迟 P99 | 100 并发 | 10 万条 | latency p99 | < 5ms |
| 读取吞吐 (全表扫描) | 20 并发 | 100 万条 | reads/sec | > 10K/sec |
| 读取延迟 (前缀迭代) | 20 并发 | 10 万条 | latency p99 | < 10ms |
| 聚合计算延迟 | 10 并发 | 50 万条已聚合 | agg latency | < 50ms (日维度) |
| 压缩效率 | - | 100 万条原始数据 | compression ratio | > 5:1 (float), > 10:1 (timestamp) |
| 内存占用稳定 | 100 并发持续 30min | 连续写入 | RSS growth | < 10% (无内存泄漏) |
| WAL rotate 性能 | 1000 ops/sec | 持续写入 | rotate latency | < 1ms |
| SkipList 大数据量 | 100 万节点 | range_query | query latency | < 1ms |
| InvertedIndex bitmap 操作 | 100 万 series | intersection | op latency | < 0.5ms |

#### 3.2 性能基准测试框架扩展

在 `crates/tsdb-cli/src/bin/bench.rs` 中新增：

```rust
// K8s 监控场景压力测试
fn bench_k8s_metrics_ingestion(writers: usize, duration_secs: u64) -> BenchResult {
    let mut handles = vec![];
    let total_ops = Arc::new(AtomicU64::new(0));
    let start = Instant::now();

    for w in 0..writers {
        let ops = total_ops.clone();
        let engine = engine_clone(); // 每个 writer 独立连接
        handles.push(thread::spawn(move || {
            let rng = StdRng::seed_from_u64(w as u64);
            while start.elapsed().as_secs() < duration_secs {
                let pod_id = rng.gen_range(0..5000);
                let ns = ["default", "kube-system", "monitoring"][rng.gen_range(0..3)];
                let mut dp = DataPoint::new("container_metrics", now_micros());
                dp.tags.insert("pod".to_string(), format!("pod-{}", pod_id));
                dp.tags.insert("namespace".to_string(), ns.to_string());
                dp.fields.insert("cpu".to_string(), FieldValue::Float(rng.gen::<f64>() * 100.0));
                dp.fields.insert("mem".to_string(), FieldValue::Integer(rng.gen_range(0..8192)));
                engine.write(&dp).ok();
                ops.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    for h in handles { h.join().ok(); }
    let elapsed = start.elapsed();
    BenchResult {
        total_ops: total_ops.load(Ordering::Relaxed),
        ops_per_sec: total_ops.load(Ordering::Relaxed) as f64 / elapsed.as_secs_f64(),
        elapsed,
    }
}
```

#### 3.3 CI 中集成性能回归检测

在 `.github/workflows/ci.yml` 新增 `perf` job：

```yaml
perf:
  name: Performance Regression
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - run: sudo apt-get update && sudo apt-get install -y libclang-dev librocksdb-dev
    - name: Build release
      run: cargo build --release -p tsdb-cli
    - name: Run benchmarks
      run: cargo run --release --bin tsdb-cli -- bench --regression
    - name: Check regression
      run: |
        # 如果性能下降超过 10%，CI 失败
        python scripts/check_perf_regression.py benchmark_results.json
```

#### 3.4 验收标准

| 指标 | 目标值 |
|------|--------|
| 写入吞吐 (单线程) | > 50K ops/s |
| 写入吞吐 (100线程) | > 200K ops/s |
| 写入延迟 P99 | < 5ms |
| 读取延迟 P99 | < 10ms |
| 压缩比 (float) | > 5:1 |
| 内存泄漏 | 无 (RSS 稳定) |
| 性能回退阈值 | < 10% |

---

## Phase 4: 系统部署

### 目标

将 TSDB 打包为可部署的生产级服务。

#### 4.1 Docker 化

**Dockerfile (multi-stage build)**:
```dockerfile
# Stage 1: Build
FROM rust:1.85-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release -p tsdb-server

# Stage 2: Runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates librocksdb-dev && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/tsdb-server /usr/local/bin/
EXPOSE 8080 9000 9100 9200
ENTRYPOINT ["tsdb-server"]
CMD ["--config", "/etc/tsdb/tsdb.ini"]
```

**docker-compose.yml**:
```yaml
version: '3.8'
services:
  tsdb-server:
    build: .
    ports:
      - "8080:8080"   # HTTP API
      - "9000:9000"   # TCP (binary protocol)
      - "9100:9100"   # NNG REP
      - "9200:9200"   # NNG PUB
    volumes:
      - tsdb-data:/data/tsdb
      - ./tsdb.ini:/etc/tsdb/tsdb.ini:ro
    environment:
      - RUST_LOG=tsdb_server=info
    deploy:
      resources:
        limits:
          memory: 4G
          cpus: '2'
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 10s
      timeout: 5s
      retries: 3

volumes:
  tsdb-data:
```

#### 4.2 Kubernetes 部署

**deployment.yaml**:
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: tsdb-server
spec:
  replicas: 3
  selector:
    matchLabels:
      app: tsdb-server
  template:
    spec:
      containers:
      - name: tsdb-server
        image: ghcr.io/supermy/tsdb:latest
        ports:
        - containerPort: 8080
        - containerPort: 9000
        resources:
          requests:
            memory: "512Mi"
            cpu: "250m"
          limits:
            memory: "4Gi"
            cpu: "2000m"
        volumeMounts:
        - name: data
          mountPath: /data/tsdb
        livenessProbe:
          httpGet:
            path: /health
            port: 8080
          initialDelaySeconds: 10
          periodSeconds: 10
        readinessProbe:
          httpGet:
            path: /health
            port: 8080
          initialDelaySeconds: 5
          periodSeconds: 5
      volumes:
      - name: data
        persistentVolumeClaim:
          claimName: tsdb-pvc
---
apiVersion: v1
kind: Service
metadata:
  name: tsdb-service
spec:
  type: LoadBalancer
  ports:
  - port: 8080
    targetPort: 8080
  - port: 9000
    targetPort: 9000
  selector:
    app: tsdb-server
```

#### 4.3 配置管理

**生产级 tsdb.ini**:
```ini
[server]
host = 0.0.0.0
port = 8080
workers = 4

[storage]
data_dir = /data/tsdb
hot_days = 7
retention_days = 90
block_duration_secs = 30
write_buffer_size = 67108864
max_open_files = 10000

[aggregate]
enabled = true
worker_count = 4
time_dimensions = ["hour", "day", "week", "month"]

[log]
level = info
file = /var/log/tsdb/server.log
```

#### 4.4 验收标准

| 指标 | 目标 |
|------|------|
| Docker 镜像大小 | < 50MB (stripped) |
| 冷启动时间 | < 5 秒 |
| 健康检查 | GET /health 返回 200 |
| 优雅关闭 | SIGTERM → 10s 内完成刷盘 |
| 数据持久化 | 重启后数据不丢失 |

---

## Phase 5: 安全测试

### 目标

发现和修复安全漏洞。

### 业务数据集 D: 安全攻击模拟

模拟 **恶意输入注入** 场景：
- SQL 注入尝试 (`' OR 1=1; DROP TABLE--`)
- XSS payload (`<script>alert('xss')</script>`)
- 超长字符串 (1MB field value)
- 特殊字符 (`\x00`, `\n`, null bytes)
- Unicode 绕过 (`Ｓｅｌｅｃｔ * FROM users`)

#### 5.1 安全测试清单

| # | 测试类别 | 测试项 | 文件 | 测试数 |
|---|---------|--------|------|--------|
| S1 | 输入验证 | SQL 注入防护 | parser.rs | 5 |
| S2 | 输入验证 | 超长字段名/值拒绝 | engine.rs, http_api.rs | 4 |
| S3 | 输入验证 | 特殊字符转义 | svg.rs, rowkey.rs | 3 |
| S4 | 认证鉴权 | 无认证接口保护 | http_api.rs | 3 |
| S5 | TLS | HTTPS 支持 | server.rs | 2 |
| S6 | 权限控制 | CreateDB/DropDB 权限 | http_api.rs | 3 |
| S7 | 速率限制 | 写入 QPS 限制 | http_api.rs | 2 |
| S8 | 敏感信息 | 密码/Token 不泄露日志 | protocol.rs, error.rs | 2 |
| S9 | 依赖漏洞 | Cargo audit | Cargo.lock | 1 (automated) |

#### 5.2 安全加固实施

```rust
// http_api.rs — 输入长度限制
const MAX_MEASUREMENT_LEN: usize = 256;
const MAX_TAG_VALUE_LEN: usize = 1024;
const MAX_FIELD_VALUE_LEN: usize = 65536;

fn validate_input(measurement: &str, tags: &[(String, String)], fields: &[(String, FieldValue)]) -> Result<(), TsdbError> {
    if measurement.len() > MAX_MEASUREMENT_LEN || !measurement.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(TsdbError::InvalidDataPoint("invalid measurement name".into()));
    }
    for (k, v) in tags {
        if k.len() > 128 || v.len() > MAX_TAG_VALUE_LEN {
            return Err(TsdbError::InvalidDataPoint("tag too long".into()));
        }
    }
    Ok(())
}

// parser.rs — SQL 注入防护
fn sanitize_identifier(ident: &str) -> Option<String> {
    if ident.chars().all(|c| c.is_alphanumeric() || c == '_') {
        Some(ident.to_string())
    } else {
        None
    }
}
```

#### 5.3 CI 集成安全扫描

```yaml
security:
  name: Security Audit
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - name: Cargo audit
      uses: actions-rs/audit-check@v1
      continue-on-error: true
    - name: Dependency review
      uses: actions/dependency-review-action@v3
```

#### 5.4 验收标准

| 指标 | 目标 |
|------|------|
| OWASP Top 10 覆盖 | 注入/XSS/认证/加密/访问控制 |
| CVE 数量 (依赖) | 0 HIGH/CRITICAL |
| 输入边界检查 | 100% 公开 API |
| TLS 支持 | 可选启用 |
| 速率限制 | 可配置 QPS 上限 |

---

## Phase 6: 上线联调 + 灰度发布

### 目标

在生产环境逐步放量，验证真实负载下系统稳定性。

### 业务数据集 E: 生产流量复制

从现有 Prometheus/Grafana 或 InfluxDB 导出真实查询模式：
- Top 10 高频 measurement
- 实际查询 QPS 分布
- 典型的 GROUP BY + WHERE + 时间范围组合

#### 6.1 灰度发布策略

```
Step 1: canary (5% 流量)  → 观察 24h
Step 2: staging (20% 流量) → 观察 48h
Step 3: production (100%)  → 持续监控
```

#### 6.2 上线检查清单

| # | 检查项 | 验证方式 | 状态 |
|---|--------|---------|------|
| D1 | 配置正确 (端口/路径/权限) | diff staging vs prod config | ☐ |
| D2 | 数据目录挂载 + 权限 | ls -la /data/tsdb | ☐ |
| D3 | RocksDB 版本兼容 | db.properties 验证 | ☐ |
| D4 | 日志输出正常 | tail -f /var/log/tsdb/server.log | ☐ |
| D5 | 健康检查通过 | curl localhost:8080/health | ☐ |
| D6 | 写入一条测试数据 | POST /api/v1/write | ☐ |
| D7 | 读取刚写入的数据 | GET /api/v1/query?sql=... | ☐ |
| D8 | 聚合管道运行正常 | 检查 AggregationStore 数据 | ☐ |
| D9 | 索引持久化正常 | 重启后索引可恢复 | ☐ |
| D10 | 资源使用正常 | top/htop CPU/MEM/IO | ☐ |
| D11 | 告警规则配置 | Prometheus + Alertmanager | ☐ |
| D12 | 回滚方案就绪 | docker rollback script | ☐ |

#### 6.3 监控指标埋点

```rust
// 在关键路径添加 metrics 计数
use prometheus::{IntCounter, Histogram};

lazy_static! {
    static ref WRITE_OPS_TOTAL: IntCounter =
        register_int_counter!("tsdb_write_ops_total", "Total write operations").unwrap();
    static ref WRITE_LATENCY_HISTOGRAM: Histogram =
        register_histogram!("tsdb_write_latency_seconds", "Write latency").unwrap();
    static ref READ_OPS_TOTAL: IntCounter =
        register_int_counter!("tsdb_read_ops_total", "Total read operations").unwrap();
}

// StorageEngine::write()
pub fn write(&self, dp: &DataPoint) -> Result<()> {
    let timer = WRITE_LATENCY_HISTOGRAM.start_timer();
    let result = self.write_internal(dp);
    if result.is_ok() { WRITE_OPS_TOTAL.inc(); }
    timer.observe_duration();
    result
}
```

#### 6.4 验收标准

| 指标 | canary | staging | prod |
|------|--------|---------|------|
| 流量占比 | 5% | 20% | 100% |
| 错误率 | < 0.1% | < 0.05% | < 0.01% |
| P99 延迟 | < 50ms | < 30ms | < 20ms |
| 可用性 | > 99.9% | > 99.95% | > 99.99% |
| 回滚时间 | < 5min | < 10min | < 15min |

---

## Phase 7: 生产运维

### 目标

建立完善的运维体系，确保长期稳定运行。

#### 7.1 运维工具集

| 工具 | 用途 | 实现方式 |
|------|------|---------|
| tsdb-admin | 管理 CLI (backup/restore/compact/stats) | tsdb-cli 子命令 |
| tsdb-exporter | Prometheus exporter (/metrics) | warp 端点 |
| tsdb-backup | 定期快照 + WAL 归档 | cron + rocksdb sst_dump |
| tsdb-monitor | Grafana dashboard JSON | tsdb-chart 渲染 |

#### 7.2 故障处理 SOP

| 故障现象 | 可能原因 | 处理步骤 |
|---------|---------|---------|
| 写入延迟突增 | RocksDB STALL | 检查 memtable count, 增加 write_buffer |
| 读取超时 | SST 文件过多 | 触发 compaction, 增加 level0_slowdown |
| OOM kill | block cache 过大 | 降低 block_cache 大小, 优化压缩 |
| 磁盘满 | SST/WAL 占用过多 | cleanup_expired_cfs, 增大 retention |
| CRC 不匹配 | 磁盘 I/O 错误 | 检查 dmesg, 替换磁盘 |

#### 7.3 验收标准

| 指标 | 目标 |
|------|------|
| MTTR (平均恢复时间) | < 15 分钟 |
| RPO (恢复点目标) | < 1 分钟 (WAL) |
| 备份频率 | 每小时增量 + 每天全量 |
| Grafana Dashboard | CPU/MEM/DISK/QPS/LATENCY/ERRORS |
| 告警响应 | P1: 5min, P2: 30min, P3: 4h |

---

## 各阶段业务数据集总结

| 阶段 | 数据集 | 业务场景 | 核心特征 |
|------|--------|---------|---------|
| Phase 1 | **A: IoT 设备监控** | 智能工厂 | 100 设备, 10s 间隔, 多 tag 维度 |
| Phase 2 | **B: 金融交易流水** | 支付网关 | 高频写入, ACID, 时间窗聚合 |
| Phase 3 | **C: K8s 集群监控** | 容器平台 | 10万QPS, 5000 pod, 海量数据 |
| Phase 5 | **D: 安全攻击模拟** | 渗透测试 | 恶意输入, 边界值, 注入攻击 |
| Phase 6 | **E: 生产流量复制** | 真实负载 | 实际查询模式, Top-N measurement |

---

## 总体时间规划

```
Week 1-2:   Phase 1 — 功能开发 + 单元测试 (TDD)
Week 3:     Phase 2 — 业务联调 + 集成测试
Week 4:     Phase 3 — 压力性能测试 + 瓶颈优化
Week 5:     Phase 4 — Docker/K8s 部署方案
Week 5-6:   Phase 5 — 安全测试 + 加固
Week 6-7:   Phase 6 — 灰度发布 + 上线联调
Week 7+:    Phase 7 — 生产运维 + 持续优化
```

---

## 成功定义

| 维度 | Phase 1 入口 | 最终目标 |
|------|-------------|---------|
| 测试数量 | 146 | > 300 (单元+集成+性能+安全) |
| 代码质量 | 0 warning | 0 warning + 安全审计通过 |
| 功能完备度 | 核心 70% | 95%+ (含部署/监控/运维) |
| 性能基线 | 未建立 | 基准入库 + 回归检测 |
| 部署能力 | 仅 cargo run | Docker + K8s 一键部署 |
| 安全 posture | 无防护 | OWASP 合规 + TLS 可选 |
| 生产就绪 | 否 | 灰度发布 + 99.99% SLA |
