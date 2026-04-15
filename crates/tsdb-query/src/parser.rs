use sqlparser::dialect::GenericDialect;
use sqlparser::ast::{Statement, Query, SetExpr, SelectItem, Expr, BinaryOperator, Value};
use sqlparser::parser::Parser;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct ParsedQuery {
    pub measurement: String,
    pub select_fields: Vec<SelectField>,
    pub where_clause: Option<WhereClause>,
    pub group_by: Vec<GroupByExpr>,
    pub order_by: Option<OrderByExpr>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub enum SelectField {
    Star,
    Field(String),
    Aggregate { func: AggFunc, field: String, alias: Option<String> },
}

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

#[derive(Debug, Clone)]
pub struct WhereClause {
    pub time_range: Option<(i64, i64)>,
    pub tag_filters: Vec<(String, String, FilterOp)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterOp {
    Eq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
}

#[derive(Debug, Clone)]
pub enum GroupByExpr {
    Time { interval: i64 },
    Tag(String),
}

#[derive(Debug, Clone)]
pub struct OrderByExpr {
    pub field: String,
    pub desc: bool,
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("SQL parse error: {0}")]
    SqlParse(String),
    #[error("unsupported query: {0}")]
    Unsupported(String),
    #[error("invalid measurement: {0}")]
    InvalidMeasurement(String),
}

pub struct SqlParser {
    dialect: GenericDialect,
}

impl SqlParser {
    pub fn new() -> Self {
        Self {
            dialect: GenericDialect {},
        }
    }

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

    fn parse_where(&self, expr: &Expr) -> Result<WhereClause, ParseError> {
        let mut time_range = None;
        let mut tag_filters = Vec::new();
        self.extract_filters(expr, &mut time_range, &mut tag_filters)?;
        Ok(WhereClause { time_range, tag_filters })
    }

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

    fn parse_group_by_expr(&self, expr: &Expr) -> Result<GroupByExpr, ParseError> {
        if let Expr::Identifier(ident) = expr {
            return Ok(GroupByExpr::Tag(ident.value.clone()));
        }
        Err(ParseError::Unsupported(format!("unsupported GROUP BY: {:?}", expr)))
    }

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
