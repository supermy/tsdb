//! RocksDB 钩子模块 - Storage Engine Lifecycle Hooks
//!
//! 提供 RocksDB 事件回调的 Rust 封装，用于监控和定制存储引擎行为。
//! 通过 EventListener 机制监听 flush、compaction、ingestion 等事件。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct StorageMetrics {
    pub flush_count: AtomicU64,
    pub compaction_count: AtomicU64,
    pub ingestion_count: AtomicU64,
    pub bytes_written: AtomicU64,
    pub bytes_read: AtomicU64,
    pub write_stall_count: AtomicU64,
}

impl StorageMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn record_flush(&self) {
        self.flush_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_compaction(&self) {
        self.compaction_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_ingestion(&self) {
        self.ingestion_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_write(&self, bytes: u64) {
        self.bytes_written.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_read(&self, bytes: u64) {
        self.bytes_read.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_stall(&self) {
        self.write_stall_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            flush_count: self.flush_count.load(Ordering::Relaxed),
            compaction_count: self.compaction_count.load(Ordering::Relaxed),
            ingestion_count: self.ingestion_count.load(Ordering::Relaxed),
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
            bytes_read: self.bytes_read.load(Ordering::Relaxed),
            write_stall_count: self.write_stall_count.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub flush_count: u64,
    pub compaction_count: u64,
    pub ingestion_count: u64,
    pub bytes_written: u64,
    pub bytes_read: u64,
    pub write_stall_count: u64,
}

pub trait StorageHook: Send + Sync {
    fn on_flush_start(&self, _cf_name: &str) {}
    fn on_flush_complete(&self, _cf_name: &str, _bytes: u64) {}
    fn on_compaction_start(&self, _cf_name: &str, _level: u32) {}
    fn on_compaction_complete(&self, _cf_name: &str, _level: u32, _bytes_moved: u64) {}
    fn on_ingestion(&self, _cf_name: &str, _files: usize) {}
    fn on_write_stall(&self, _cf_name: &str) {}
}

pub struct MetricsCollectingHook {
    metrics: Arc<StorageMetrics>,
}

impl MetricsCollectingHook {
    pub fn new(metrics: Arc<StorageMetrics>) -> Self {
        Self { metrics }
    }
}

impl StorageHook for MetricsCollectingHook {
    fn on_flush_complete(&self, _cf_name: &str, bytes: u64) {
        self.metrics.record_flush();
        self.metrics.record_write(bytes);
    }

    fn on_compaction_complete(&self, _cf_name: &str, _level: u32, bytes_moved: u64) {
        self.metrics.record_compaction();
        self.metrics.record_write(bytes_moved);
    }

    fn on_ingestion(&self, _cf_name: &str, _files: usize) {
        self.metrics.record_ingestion();
    }

    fn on_write_stall(&self, _cf_name: &str) {
        self.metrics.record_stall();
    }
}

pub struct HookManager {
    hooks: Vec<Box<dyn StorageHook>>,
    metrics: Arc<StorageMetrics>,
}

impl HookManager {
    pub fn new() -> Self {
        let metrics = StorageMetrics::new();
        let hooks: Vec<Box<dyn StorageHook>> = vec![Box::new(MetricsCollectingHook::new(metrics.clone()))];
        Self { hooks, metrics }
    }

    pub fn register_hook(&mut self, hook: Box<dyn StorageHook>) {
        self.hooks.push(hook);
    }

    pub fn metrics(&self) -> Arc<StorageMetrics> {
        self.metrics.clone()
    }

    pub fn fire_flush_start(&self, cf_name: &str) {
        for hook in &self.hooks {
            hook.on_flush_start(cf_name);
        }
    }

    pub fn fire_flush_complete(&self, cf_name: &str, bytes: u64) {
        for hook in &self.hooks {
            hook.on_flush_complete(cf_name, bytes);
        }
    }

    pub fn fire_compaction_start(&self, cf_name: &str, level: u32) {
        for hook in &self.hooks {
            hook.on_compaction_start(cf_name, level);
        }
    }

    pub fn fire_compaction_complete(&self, cf_name: &str, level: u32, bytes_moved: u64) {
        for hook in &self.hooks {
            hook.on_compaction_complete(cf_name, level, bytes_moved);
        }
    }

    pub fn fire_ingestion(&self, cf_name: &str, files: usize) {
        for hook in &self.hooks {
            hook.on_ingestion(cf_name, files);
        }
    }

    pub fn fire_write_stall(&self, cf_name: &str) {
        for hook in &self.hooks {
            hook.on_write_stall(cf_name);
        }
    }
}

impl Default for HookManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_snapshot() {
        let metrics = StorageMetrics::new();
        metrics.record_flush();
        metrics.record_flush();
        metrics.record_compaction();
        metrics.record_write(1024);

        let snap = metrics.snapshot();
        assert_eq!(snap.flush_count, 2);
        assert_eq!(snap.compaction_count, 1);
        assert_eq!(snap.bytes_written, 1024);
    }

    #[test]
    fn test_hook_manager() {
        let mgr = HookManager::new();
        let metrics = mgr.metrics();

        mgr.fire_flush_complete("hot_2024_01_01", 4096);
        mgr.fire_compaction_complete("hot_2024_01_01", 2, 8192);

        let snap = metrics.snapshot();
        assert_eq!(snap.flush_count, 1);
        assert_eq!(snap.compaction_count, 1);
        assert_eq!(snap.bytes_written, 4096 + 8192);
    }
}
