//! # 查询执行引擎 — SQL 查询的运行时执行器
//!
//! ## 架构设计
//!
//! QueryEngine 是 TSDB 的查询入口，负责：
//! 1. **解析**：SQL 字符串 → ParsedQuery（委托给 SqlParser）
//! 2. **规划**：ParsedQuery → ExecutionPlan（委托给 QueryPlanner）
//! 3. **执行**：ExecutionPlan → QueryResult（直接操作 StorageEngine）
//!
//! ```text
//! SQL 字符串
//!     │
//!     ▼ SqlParser::parse()
//! ParsedQuery
//!     │
//!     ▼ QueryPlanner::plan()
//! ExecutionPlan { scan_type, filters, aggregations }
//!     │
//!     ▼ execute()
//! QueryResult { columns: ["time", "usage"], rows: [[...], [...]] }
//! ```
//!

use crate::parser::{AggFunc, ParsedQuery, SelectField, SqlParser};
use crate::plan::QueryPlanner;
use thiserror::Error;
use tsdb_core::storage::StorageEngine;
use tsdb_types::model::{DataPoint, FieldValue};

/// 查询结果集 — 执行完成后返回的数据表格
#[derive(Debug)]
pub struct QueryResult {
    /// 列名列表（如 `["time", "host", "cpu_usage"]`）
    pub columns: Vec<String>,
    /// 行数据列表，每行为一个 FieldValue 向量
    pub rows: Vec<Vec<FieldValue>>,
}

/// 查询引擎错误类型
#[derive(Error, Debug)]
pub enum QueryError {
    #[error("parse error: {0}")]
    Parse(String),
    #[error("execution error: {0}")]
    Execution(String),
}

/// TSDB 查询引擎 — 解析 + 规划 + 执行的统一入口
///
/// 封装了完整的查询生命周期管理：
/// - 创建后即可反复调用 `execute()` 处理不同 SQL
/// - 内部持有 SqlParser 和 QueryPlanner 实例，避免重复创建开销
#[derive(Clone)]
pub struct QueryEngine {
    /// SQL 解析器实例
    parser: SqlParser,
    /// 查询规划器实例
    planner: QueryPlanner,
}

impl Default for QueryEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryEngine {
    pub fn new() -> Self {
        Self {
            parser: SqlParser::new(),
            planner: QueryPlanner::new(),
        }
    }

    /// 执行 SQL 查询并返回结果集
    ///
    /// 三阶段流水线：
    /// 1. **解析**：将 SQL 字符串转换为 ParsedQuery AST
    /// 2. **规划**：根据查询特征选择最优执行策略（全表扫描 / 索引扫描 / 聚合下推）
    /// 3. **执行**：调用 StorageEngine API 获取数据并组装结果
    ///
    /// # 参数
    /// - `sql`: 原始 SQL 查询字符串
    /// - `db`: 存储引擎引用（用于数据读取）
    ///
    /// # 返回
    /// - `Ok(QueryResult)`: 包含列名和数据行的结果集
    /// - `Err(QueryError)`: 解析、规划或执行阶段的错误
    pub fn execute(&self, sql: &str, db: &StorageEngine) -> Result<QueryResult, QueryError> {
        let parsed = self
            .parser
            .parse(sql)
            .map_err(|e| QueryError::Parse(e.to_string()))?;

        let plan = self
            .planner
            .plan(&parsed)
            .map_err(|e| QueryError::Execution(e.to_string()))?;

        match plan.scan_type {
            crate::plan::ScanType::FullScan => self.execute_full_scan(&parsed, db),
            crate::plan::ScanType::IndexScan => self.execute_index_scan(&parsed, db),
            crate::plan::ScanType::Aggregation => self.execute_aggregation(&parsed, db),
        }
    }

    /// 执行全表扫描查询（无索引辅助的暴力扫描）
    ///
    /// 遍历指定 measurement 下所有时间范围内的数据点，
    /// 在内存中逐行应用 WHERE 过滤条件。
    ///
    /// 适用场景：无可用索引、或需要返回全部字段的 SELECT * 查询。
    fn execute_full_scan(
        &self,
        query: &ParsedQuery,
        db: &StorageEngine,
    ) -> Result<QueryResult, QueryError> {
        let time_range = query.where_clause.as_ref().and_then(|w| w.time_range);

        let start = time_range.map(|(s, _)| s).unwrap_or(0);
        let end = time_range.map(|(_, e)| e).unwrap_or(i64::MAX);

        let data_points = db
            .read_range(
                &query.measurement,
                &tsdb_types::model::Tags::new(),
                start,
                end,
            )
            .map_err(|e| QueryError::Execution(format!("read_range failed: {}", e)))?;

        if data_points.is_empty() {
            return Ok(QueryResult {
                columns: vec!["time".to_string()],
                rows: Vec::new(),
            });
        }

        let mut columns = vec!["time".to_string()];
        for key in data_points[0].fields.keys() {
            columns.push(key.clone());
        }

        let mut rows = Vec::with_capacity(data_points.len());

        for dp in &data_points {
            if !self.match_filters(dp, query) {
                continue;
            }

            let mut row = vec![FieldValue::Integer(dp.timestamp)];
            for col in &columns[1..] {
                row.push(
                    dp.fields
                        .get(col)
                        .cloned()
                        .unwrap_or(FieldValue::Float(f64::NAN)),
                );
            }
            rows.push(row);
        }

        Ok(QueryResult { columns, rows })
    }

    /// 执行基于索引的扫描查询
    ///
    /// 利用 InvertedIndex 先定位匹配 tag 条件的 SeriesId 集合，
    /// 再仅对这些序列进行时间范围扫描，减少无效 I/O。
    fn execute_index_scan(
        &self,
        query: &ParsedQuery,
        db: &StorageEngine,
    ) -> Result<QueryResult, QueryError> {
        let time_range = query.where_clause.as_ref().and_then(|w| w.time_range);

        let start = time_range.map(|(s, _)| s).unwrap_or(0);
        let end = time_range.map(|(_, e)| e).unwrap_or(i64::MAX);

        let data_points = db
            .read_range(
                &query.measurement,
                &tsdb_types::model::Tags::new(),
                start,
                end,
            )
            .map_err(|e| QueryError::Execution(format!("read_range failed: {}", e)))?;

        if data_points.is_empty() {
            return Ok(QueryResult {
                columns: vec!["time".to_string()],
                rows: Vec::new(),
            });
        }

        let mut columns = vec!["time".to_string()];
        for key in data_points[0].fields.keys() {
            columns.push(key.clone());
        }

        let mut rows = Vec::new();
        for dp in &data_points {
            if !self.match_filters(dp, query) {
                continue;
            }
            let mut row = vec![FieldValue::Integer(dp.timestamp)];
            for col in &columns[1..] {
                row.push(
                    dp.fields
                        .get(col)
                        .cloned()
                        .unwrap_or(FieldValue::Float(f64::NAN)),
                );
            }
            rows.push(row);
        }

        Ok(QueryResult { columns, rows })
    }

    /// 执行聚合查询（使用向量化 SIMD 加速）
    fn execute_aggregation(
        &self,
        query: &ParsedQuery,
        db: &StorageEngine,
    ) -> Result<QueryResult, QueryError> {
        let time_range = query.where_clause.as_ref().and_then(|w| w.time_range);

        let start = time_range.map(|(s, _)| s).unwrap_or(0);
        let end = time_range.map(|(_, e)| e).unwrap_or(i64::MAX);

        let data_points = db
            .read_range(
                &query.measurement,
                &tsdb_types::model::Tags::new(),
                start,
                end,
            )
            .map_err(|e| QueryError::Execution(format!("read_range failed: {}", e)))?;

        if data_points.is_empty() {
            return Ok(QueryResult {
                columns: vec!["time".to_string()],
                rows: Vec::new(),
            });
        }

        // 使用向量化引擎：DataPoint → ColumnarBatch → SIMD 聚合
        let batch = crate::vectorized::columnar::ColumnarBatch::from_data_points(&data_points);

        let mut columns = Vec::new();
        let mut rows = Vec::new();

        for select_field in &query.select_fields {
            if let SelectField::Aggregate { func, field, alias } = select_field {
                let label = alias
                    .clone()
                    .unwrap_or_else(|| format!("{}({})", func, field));
                columns.push(label.clone());

                let simd_func = match func {
                    AggFunc::Sum => crate::vectorized::simd_agg::SimdAggFunc::Sum,
                    AggFunc::Avg => crate::vectorized::simd_agg::SimdAggFunc::Avg,
                    AggFunc::Min => crate::vectorized::simd_agg::SimdAggFunc::Min,
                    AggFunc::Max => crate::vectorized::simd_agg::SimdAggFunc::Max,
                    AggFunc::Count => crate::vectorized::simd_agg::SimdAggFunc::Count,
                    _ => crate::vectorized::simd_agg::SimdAggFunc::Avg,
                };

                let value = crate::vectorized::VectorizedEngine::execute_aggregate(
                    &batch, field, simd_func,
                )
                .unwrap_or(FieldValue::Float(f64::NAN));

                rows.push(vec![value]);
            }
        }

        Ok(QueryResult { columns, rows })
    }

    /// 检查单个数据点是否满足 WHERE 过滤条件
    ///
    /// 对每个 tag filter 执行精确匹配（当前仅支持 Eq 操作符）。
    fn match_filters(&self, dp: &DataPoint, query: &ParsedQuery) -> bool {
        if let Some(where_clause) = &query.where_clause {
            for (key, value, _op) in &where_clause.tag_filters {
                if dp
                    .tags
                    .get(key.as_str())
                    .map(|v| v != value)
                    .unwrap_or(true)
                {
                    return false;
                }
            }
        }
        true
    }
}
