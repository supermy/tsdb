//! # SQL 解析器 — 将 SQL 字符串转换为结构化查询对象
//!
//! ## 功能概述
//!
//! 基于 `sqlparser` crate 实现 TSDB 方言的 SQL 解析，支持以下语法：
//!
//! ```sql
//! -- 简单查询
//! SELECT * FROM cpu WHERE host='server01' AND time > 1713158400000000
//!
//! -- 聚合查询
//! SELECT AVG(usage), MAX(usage) FROM cpu GROUP BY host ORDER BY time DESC LIMIT 100
//! ```
//!
//! ## 解析流程
//!
//! ```text
//! SQL 字符串
//!     │
//!     ▼ (sqlparser 解析)
//! sqlparser::ast::Statement
//!     │
//!     ▼ (语义转换)
//! ParsedQuery {
//!     measurement: "cpu",
//!     select_fields: [Aggregate { func: Avg, field: "usage" }],
//!     where_clause: Some { time_range, tag_filters },
//!     group_by: [Tag("host")],
//!     order_by: Some { field: "time", desc: true },
//!     limit: Some(100),
//! }
//! ```
//!

use sqlparser::dialect::GenericDialect;
use sqlparser::ast::{Statement, Query, SetExpr, SelectItem, Expr, BinaryOperator, Value};
use sqlparser::parser::Parser;
use thiserror::Error;

/// 解析后的查询结构 — SQL → ParsedQuery 的中间表示
#[derive(Debug, Clone)]
pub struct ParsedQuery {
    /// 目标 measurement（表名），如 `"cpu"`, `"memory"`
    pub measurement: String,
    /// SELECT 子句中的字段列表（支持通配符、普通字段、聚合函数）
    pub select_fields: Vec<SelectField>,
    /// WHERE 条件（时间范围 + 标签过滤）
    pub where_clause: Option<WhereClause>,
    /// GROUP BY 表达式列表（按时间维度或标签分组）
    pub group_by: Vec<GroupByExpr>,
    /// ORDER BY 排序表达式
    pub order_by: Option<OrderByExpr>,
    /// LIMIT 返回行数限制
    pub limit: Option<usize>,
}

/// SELECT 字段类型枚举
#[derive(Debug, Clone)]
pub enum SelectField {
    /// 通配符 `SELECT *`
    Star,
    /// 普通字段名 `SELECT usage`
    Field(String),
    /// 聚合函数 `SELECT AVG(usage) AS avg_usage`
    Aggregate { func: AggFunc, field: String, alias: Option<String> },
}

/// 支持的聚合函数类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AggFunc {
    Sum,
    Avg,
    Min,
    Max,
    Count,
    First,
    Last,
}

impl std::fmt::Display for AggFunc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AggFunc::Sum => write!(f, "SUM"),
            AggFunc::Avg => write!(f, "AVG"),
            AggFunc::Min => write!(f, "MIN"),
            AggFunc::Max => write!(f, "MAX"),
            AggFunc::Count => write!(f, "COUNT"),
            AggFunc::First => write!(f, "FIRST"),
            AggFunc::Last => write!(f, "LAST"),
        }
    }
}

/// WHERE 子句解析结果
#[derive(Debug, Clone)]
pub struct WhereClause {
    /// 时间范围过滤 `(起始时间戳, 结束时间戳)`
    pub time_range: Option<(i64, i64)>,
    /// 标签过滤条件列表 `(key, value, 操作符)`
    pub tag_filters: Vec<(String, String, FilterOp)>,
}

/// 过滤操作符类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterOp {
    Eq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
}

/// GROUP BY 表达式类型
#[derive(Debug, Clone)]
pub enum GroupByExpr {
    /// 按时间维度分组（如按小时/天）
    Time { interval: i64 },
    /// 按标签分组
    Tag(String),
}

/// ORDER BY 排序表达式
#[derive(Debug, Clone)]
pub struct OrderByExpr {
    /// 排序字段名
    pub field: String,
    /// 是否降序排列
    pub desc: bool,
}

/// SQL 解析错误类型
#[derive(Error, Debug)]
pub enum ParseError {
    #[error("SQL parse error: {0}")]
    SqlParse(String),
    #[error("unsupported query: {0}")]
    Unsupported(String),
    #[error("invalid measurement: {0}")]
    InvalidMeasurement(String),
}

/// SQL 解析器实例
///
/// 使用 GenericDialect（通用 SQL 方言）作为底层解析引擎，
/// 将标准 SQL AST 转换为 TSDB 专用的 ParsedQuery 结构。
pub struct SqlParser {
    dialect: GenericDialect,
}

impl SqlParser {
    /// 创建新的 SQL 解析器实例
    pub fn new() -> Self {
        Self {
            dialect: GenericDialect {},
        }
    }

    /// 解析 SQL 字符串为 ParsedQuery 结构
    ///
    /// 仅支持单条 SELECT 语句，不支持 INSERT/UPDATE/DELETE 等。
    ///
    /// # 参数
    /// - `sql`: 原始 SQL 查询字符串
    ///
    /// # 返回
    /// - `Ok(ParsedQuery)`: 成功解析的结构化查询
    /// - `Err(ParseError)`: SQL 语法错误或不受支持的查询类型
    pub fn parse(&self, sql: &str) -> Result<ParsedQuery, ParseError> {
        let statement = Parser::parse_sql(&self.dialect, sql)
            .map_err(|e| ParseError::SqlParse(e.to_string()))?;

        if statement.len() != 1 {
            return Err(ParseError::Unsupported("only single statement supported".into()));
        }

        match &statement[0] {
            Statement::Query(query) => self.parse_query(query),
            _ => Err(ParseError::Unsupported("only SELECT queries supported".into())),
        }
    }

    /// 解析 Query AST 为 ParsedQuery
    fn parse_query(&self, query: &Query) -> Result<ParsedQuery, ParseError> {
        let body = &query.body;
        match body.as_ref() {
            SetExpr::Select(select) => {
                let measurement = select.from.get(0)
                    .map(|t| t.relation.to_string())
                    .ok_or_else(|| ParseError::InvalidMeasurement("no table specified".into()))?;

                let select_fields: Vec<SelectField> = select.projection.iter()
                    .map(|item| self.parse_select_item(item))
                    .collect::<Result<Vec<_>, _>>()?;

                let where_clause = select.selection.as_ref()
                    .map(|expr| self.parse_where(expr))
                    .transpose()?;

                let group_by = match &select.group_by {
                    sqlparser::ast::GroupByExpr::Expressions(exprs, _) => {
                        exprs.iter()
                            .map(|expr| self.parse_group_by_expr(expr))
                            .collect::<Result<Vec<_>, _>>()?
                    }
                    sqlparser::ast::GroupByExpr::All(_) => Vec::new(),
                };

                let order_by = query.order_by.as_ref()
                    .and_then(|ob| ob.exprs.first())
                    .map(|expr| self.parse_order_by(expr));

                let limit = query.limit.as_ref()
                    .map(|l| l.to_string().parse().unwrap_or(0));

                Ok(ParsedQuery {
                    measurement,
                    select_fields,
                    where_clause,
                    group_by,
                    order_by,
                    limit,
                })
            }
            _ => Err(ParseError::Unsupported("only SELECT queries supported".into())),
        }
    }

    /// 解析单个 SELECT 投影项
    fn parse_select_item(&self, item: &SelectItem) -> Result<SelectField, ParseError> {
        match item {
            SelectItem::Wildcard(_) => Ok(SelectField::Star),
            SelectItem::UnnamedExpr(expr) => {
                if let Expr::Function(func) = expr {
                    let func_name = func.name.to_string().to_uppercase();
                    let agg = self.parse_agg_func(&func_name)?;
                    let field = self.extract_func_arg(func);
                    Ok(SelectField::Aggregate { func: agg, field, alias: None })
                } else if let Expr::Identifier(ident) = expr {
                    Ok(SelectField::Field(ident.value.clone()))
                } else {
                    Err(ParseError::Unsupported(format!("unsupported select item: {:?}", item)))
                }
            }
            SelectItem::ExprWithAlias { expr, alias } => {
                if let Expr::Function(func) = expr {
                    let func_name = func.name.to_string().to_uppercase();
                    let agg = self.parse_agg_func(&func_name)?;
                    let field = self.extract_func_arg(func);
                    Ok(SelectField::Aggregate {
                        func: agg,
                        field,
                        alias: Some(alias.value.clone()),
                    })
                } else {
                    Err(ParseError::Unsupported(format!("unsupported select item: {:?}", item)))
                }
            }
            _ => Err(ParseError::Unsupported(format!("unsupported select item: {:?}", item))),
        }
    }

    /// 将函数名字符串映射为 AggFunc 枚举
    fn parse_agg_func(&self, name: &str) -> Result<AggFunc, ParseError> {
        match name {
            "SUM" => Ok(AggFunc::Sum),
            "AVG" => Ok(AggFunc::Avg),
            "MIN" => Ok(AggFunc::Min),
            "MAX" => Ok(AggFunc::Max),
            "COUNT" => Ok(AggFunc::Count),
            "FIRST" => Ok(AggFunc::First),
            "LAST" => Ok(AggFunc::Last),
            _ => Err(ParseError::Unsupported(format!("unknown function: {}", name))),
        }
    }

    /// 从函数参数中提取目标字段名或常量值
    fn extract_func_arg(&self, func: &sqlparser::ast::Function) -> String {
        match &func.args {
            sqlparser::ast::FunctionArguments::List(list) => {
                list.args.first()
                    .and_then(|arg| {
                        if let sqlparser::ast::FunctionArg::Unnamed(arg_expr) = arg {
                            match arg_expr {
                                sqlparser::ast::FunctionArgExpr::Expr(Expr::Identifier(ident)) => {
                                    return Some(ident.value.clone());
                                }
                                sqlparser::ast::FunctionArgExpr::Expr(Expr::Value(Value::Number(n, _))) => {
                                    return Some(n.to_string());
                                }
                                _ => {}
                            }
                        }
                        None
                    })
                    .unwrap_or_default()
            }
            _ => String::new(),
        }
    }

    /// 解析 WHERE 子句为 WhereClause 结构
    fn parse_where(&self, expr: &Expr) -> Result<WhereClause, ParseError> {
        let mut time_range = None;
        let mut tag_filters = Vec::new();
        self.extract_filters(expr, &mut time_range, &mut tag_filters)?;
        Ok(WhereClause { time_range, tag_filters })
    }

    /// 递归提取 WHERE 中的过滤条件（支持 AND 组合）
    fn extract_filters(
        &self,
        expr: &Expr,
        time_range: &mut Option<(i64, i64)>,
        tag_filters: &mut Vec<(String, String, FilterOp)>,
    ) -> Result<(), ParseError> {
        match expr {
            Expr::BinaryOp { left, op, right } => {
                match op {
                    BinaryOperator::And => {
                        self.extract_filters(left, time_range, tag_filters)?;
                        self.extract_filters(right, time_range, tag_filters)?;
                    }
                    BinaryOperator::Eq => {
                        self.extract_comparison(left, right, FilterOp::Eq, time_range, tag_filters)?;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// 提取单个比较表达式为过滤条件
    fn extract_comparison(
        &self,
        left: &Expr,
        right: &Expr,
        op: FilterOp,
        time_range: &mut Option<(i64, i64)>,
        tag_filters: &mut Vec<(String, String, FilterOp)>,
    ) -> Result<(), ParseError> {
        if let (Expr::Identifier(ident), Expr::Value(val)) = (left, right) {
            let key = ident.value.clone();
            let value = match val {
                Value::SingleQuotedString(s) => s.clone(),
                Value::Number(n, _) => n.to_string(),
                _ => return Ok(()),
            };
            if key == "time" {
                let ts = value.parse().unwrap_or(0);
                *time_range = Some((ts, ts));
            } else {
                tag_filters.push((key, value, op));
            }
        }
        Ok(())
    }

    /// 解析 GROUP BY 表达式
    fn parse_group_by_expr(&self, expr: &Expr) -> Result<GroupByExpr, ParseError> {
        if let Expr::Identifier(ident) = expr {
            return Ok(GroupByExpr::Tag(ident.value.clone()));
        }
        Err(ParseError::Unsupported(format!("unsupported GROUP BY: {:?}", expr)))
    }

    /// 解析 ORDER BY 表达式
    fn parse_order_by(&self, expr: &sqlparser::ast::OrderByExpr) -> OrderByExpr {
        OrderByExpr {
            field: expr.expr.to_string(),
            desc: !expr.asc.unwrap_or(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_select() {
        let parser = SqlParser::new();
        let result = parser.parse("SELECT * FROM cpu").unwrap();
        assert_eq!(result.measurement, "cpu");
        assert!(matches!(result.select_fields[0], SelectField::Star));
    }

    #[test]
    fn test_aggregate_query() {
        let parser = SqlParser::new();
        let result = parser.parse("SELECT AVG(cpu) FROM system WHERE host='server01'").unwrap();
        assert_eq!(result.measurement, "system");
        if let SelectField::Aggregate { func, .. } = &result.select_fields[0] {
            assert_eq!(*func, AggFunc::Avg);
        } else {
            panic!("expected aggregate");
        }
    }
}
