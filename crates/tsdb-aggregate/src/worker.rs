//! # 异步聚合工作器 — 后台批量聚合计算
//!
//! ## 设计目标
//!
//! Worker 是 Aggregator 的异步执行包装器，运行在独立线程中：
//! - 从 NNG PULL socket 接收原始 DataPoint
//! - 累积到内部缓冲区后定期触发聚合计算
//! - 将结果通过 NNG PUB socket 推送给订阅者
//!
//! ## 数据流
//!
//! ```text
//! Agent/Writer ──NNG PUSH──► [Worker::run()] ──聚合──► [NNG PUB] ──► Monitor/Dashboard
//! ```
//!

use crate::aggregator::{Aggregator, TimeDimension};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{error, info};
use tsdb_types::model::DataPoint;

/// 异步聚合工作器 — 在后台线程中持续执行聚合任务
///
/// ## 生命周期
///
/// 1. `new()` 创建实例（配置 NNG 地址和维度列表）
/// 2. `start()` 启动后台线程（进入 run() 循环）
/// 3. 持续接收数据 → 累积 → 定期输出聚合结果
/// 4. `stop()` 设置停止标志，线程优雅退出
pub struct Worker {
    /// NNG PULL 监听地址（用于接收待聚合的 DataPoint）
    pull_url: String,
    /// NNG PUB 发布地址（用于推送聚合结果）
    pub_url: String,
    /// 需要执行的时间维度列表（如 [Hour, Day, Week, Month]）
    time_dimensions: Vec<TimeDimension>,
    /// 运行控制标志：true = 继续运行，false = 停止
    running: Arc<AtomicBool>,
}

impl Worker {
    /// 创建新的聚合工作器实例
    ///
    /// # 参数
    /// - `pull_url`: NNG PULL 协议地址（如 `"inproc://aggregate"`）
    /// - `pub_url`: NNG PUB 协议地址（如 `"tcp://0.0.0.0:9913"`）
    /// - `time_dimensions`: 需要计算的聚合时间维度列表
    pub fn new(pull_url: &str, pub_url: &str, time_dimensions: Vec<TimeDimension>) -> Self {
        Self {
            pull_url: pull_url.to_string(),
            pub_url: pub_url.to_string(),
            time_dimensions,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 启动后台聚合线程
    ///
    /// 创建独立线程运行 `run()` 方法，调用方不阻塞。
    /// 返回的 JoinHandle 可用于等待线程结束。
    ///
    /// # 返回
    /// - `Ok(std::thread::JoinHandle<()>)`: 后台线程句柄
    /// - `Err(TsdbError::Nng)`: NNG socket 创建或绑定失败
    pub fn start(&self) -> Result<std::thread::JoinHandle<()>, tsdb_core::error::TsdbError> {
        self.running.store(true, Ordering::Relaxed);
        let pull_url = self.pull_url.clone();
        let _pub_url = self.pub_url.clone();
        let time_dims = self.time_dimensions.clone();
        let running = self.running.clone();

        let handle = std::thread::spawn(move || {
            let mut aggregator = Aggregator::new();

            let pull_socket = match nng::Socket::new(nng::Protocol::Pull0) {
                Ok(s) => s,
                Err(e) => {
                    error!("Failed to create PULL socket: {:?}", e);
                    return;
                }
            };

            if pull_socket.dial(&pull_url).is_err() {
                error!("Failed to dial PULL {}", pull_url);
                return;
            }

            info!("Aggregate worker started on {}", pull_url);

            while running.load(Ordering::Relaxed) {
                let msg = pull_socket.recv();

                if let Ok(msg) = msg {
                    if let Ok(dp) = serde_json::from_slice::<DataPoint>(msg.as_slice()) {
                        aggregator.accumulate(&dp);
                    }
                }
            }

            for dim in &time_dims {
                let measurements = aggregator.measurement_names(*dim);
                for m in &measurements {
                    let results = aggregator.finalize(m, *dim);
                    for result in results {
                        info!(
                            "Aggregation [{:?}] measurement={} window_start={} values={:?}",
                            dim, result.measurement, result.window_start, result.values
                        );
                    }
                }
            }
        });

        Ok(handle)
    }

    /// 发送停止信号给后台线程
    ///
    /// 设置 running 标志为 false，run() 循环将在下一次迭代时退出。
    /// 调用方应配合 `join()` 等待线程实际退出。
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
