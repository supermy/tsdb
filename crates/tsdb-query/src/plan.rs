//! # 查询规划器 — 根据查询特征选择最优执行策略
//!
//! ## 规划策略
//!
//! QueryPlanner 分析 ParsedQuery 的结构特征，选择最合适的执行路径：
//!
//! | 查询特征 | 选择策略 | 说明 |
//! |---------|----------|------|
//! | SELECT * / 无聚合 | FullScan | 全表扫描，返回原始数据 |
//! | 有 WHERE tag 条件 | IndexScan | 利用倒排索引过滤 |
//! | 有 SUM/AVG/MAX 等 | Aggregation | 聚合下推，减少数据传输 |
//!

use crate::parser::{ParsedQuery, SelectField};

/// 扫描策略枚举
#[derive(Debug, Clone)]
pub enum ScanType {
    /// 全表扫描 — 遍历所有数据点，在内存中过滤
    FullScan,
    /// 索引扫描 — 利用倒排索引定位目标序列后扫描
    IndexScan,
    /// 聚合扫描 — 在存储层完成部分聚合计算
    Aggregation,
}

/// 执行计划 — 一次查询的完整执行蓝图
///
/// 包含查询优化器选择的扫描类型和需要传递给执行器的元信息。
#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    /// 数据扫描方式（决定 execute() 走哪条代码路径）
    pub scan_type: ScanType,
    /// 是否有聚合函数（影响结果集格式）
    pub has_aggregations: bool,
}

/// 查询规划器 — ParsedQuery → ExecutionPlan 的转换器
///
/// ## 核心规则
///
/// 1. **SELECT * 或纯字段查询** → `FullScan`（最简单的路径）
/// 2. **WHERE 含 tag 过滤条件** → `IndexScan`（利用 InvertedIndex）
/// 3. **含 SUM/AVG/MIN/MAX/COUNT** → `Aggregation`（聚合下推）
///
/// TODO: 后续可扩展更复杂的规则（如基于统计信息的代价估算）。
pub struct QueryPlanner;

impl QueryPlanner {
    /// 创建新的查询规划器实例（无状态对象）
    pub fn new() -> Self { Self }

    /// 根据解析后的查询生成执行计划
    ///
    /// # 参数
    /// - `query`: SqlParser 输出的结构化查询
    ///
    /// # 返回
    /// - `Ok(ExecutionPlan)`: 最优执行策略
    /// - `Err(String)`: 无法确定执行策略
    pub fn plan(&self, query: &ParsedQuery) -> Result<ExecutionPlan, String> {
        let has_agg = query.select_fields.iter().any(|f| matches!(f, SelectField::Aggregate { .. }));
        let has_tag_filters = query.where_clause.as_ref()
            .map(|w| !w.tag_filters.is_empty())
            .unwrap_or(false);

        if has_agg {
            Ok(ExecutionPlan {
                scan_type: ScanType::Aggregation,
                has_aggregations: true,
            })
        } else if has_tag_filters {
            Ok(ExecutionPlan {
                scan_type: ScanType::IndexScan,
                has_aggregations: false,
            })
        } else {
            Ok(ExecutionPlan {
                scan_type: ScanType::FullScan,
                has_aggregations: false,
            })
        }
    }
}
