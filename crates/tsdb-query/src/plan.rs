use crate::parser::ParsedQuery;
use tsdb_types::model::{DataPoint, FieldValue};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum PlanNode {
    Scan {
        measurement: String,
        time_range: Option<(i64, i64)>,
        tag_filters: Vec<(String, String)>,
    },
    Filter {
        input: Box<PlanNode>,
        predicate: FilterPredicate,
    },
    Project {
        input: Box<PlanNode>,
        fields: Vec<String>,
    },
    Aggregate {
        input: Box<PlanNode>,
        aggs: Vec<AggExpr>,
        group_by: Vec<String>,
    },
    Sort {
        input: Box<PlanNode>,
        field: String,
        desc: bool,
    },
    Limit {
        input: Box<PlanNode>,
        count: usize,
    },
}

#[derive(Debug, Clone)]
pub struct FilterPredicate {
    pub field: String,
    pub op: FilterOp,
    pub value: FieldValue,
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
pub struct AggExpr {
    pub func: AggFunc,
    pub field: String,
    pub alias: Option<String>,
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

pub struct QueryPlanner;

impl QueryPlanner {
    pub fn plan(query: &ParsedQuery) -> PlanNode {
        let scan = PlanNode::Scan {
            measurement: query.measurement.clone(),
            time_range: query.where_clause.as_ref().and_then(|w| w.time_range),
            tag_filters: query.where_clause.as_ref()
                .map(|w| w.tag_filters.iter()
                    .filter(|(_, _, op)| *op == crate::parser::FilterOp::Eq)
                    .map(|(k, v, _)| (k.clone(), v.clone()))
                    .collect())
                .unwrap_or_default(),
        };

        let mut plan = PlanNode::Scan {
            measurement: query.measurement.clone(),
            time_range: query.where_clause.as_ref().and_then(|w| w.time_range),
            tag_filters: query.where_clause.as_ref()
                .map(|w| w.tag_filters.iter()
                    .filter(|(_, _, op)| *op == crate::parser::FilterOp::Eq)
                    .map(|(k, v, _)| (k.clone(), v.clone()))
                    .collect())
                .unwrap_or_default(),
        };

        if !query.select_fields.iter().all(|f| matches!(f, crate::parser::SelectField::Star)) {
            let fields: Vec<String> = query.select_fields.iter()
                .filter_map(|f| match f {
                    crate::parser::SelectField::Field(name) => Some(name.clone()),
                    crate::parser::SelectField::Aggregate { field, .. } => Some(field.clone()),
                    _ => None,
                })
                .collect();
            if !fields.is_empty() {
                plan = PlanNode::Project {
                    input: Box::new(plan),
                    fields,
                };
            }
        }

        let has_aggregate = query.select_fields.iter()
            .any(|f| matches!(f, crate::parser::SelectField::Aggregate { .. }));

        if has_aggregate {
            let aggs: Vec<AggExpr> = query.select_fields.iter()
                .filter_map(|f| match f {
                    crate::parser::SelectField::Aggregate { func, field, alias } => {
                        let agg_func = match func {
                            crate::parser::AggFunc::Sum => AggFunc::Sum,
                            crate::parser::AggFunc::Avg => AggFunc::Avg,
                            crate::parser::AggFunc::Min => AggFunc::Min,
                            crate::parser::AggFunc::Max => AggFunc::Max,
                            crate::parser::AggFunc::Count => AggFunc::Count,
                            crate::parser::AggFunc::First => AggFunc::First,
                            crate::parser::AggFunc::Last => AggFunc::Last,
                        };
                        Some(AggExpr {
                            func: agg_func,
                            field: field.clone(),
                            alias: alias.clone(),
                        })
                    }
                    _ => None,
                })
                .collect();

            let group_by: Vec<String> = query.group_by.iter()
                .filter_map(|g| match g {
                    crate::parser::GroupByExpr::Tag(name) => Some(name.clone()),
                    _ => None,
                })
                .collect();

            plan = PlanNode::Aggregate {
                input: Box::new(plan),
                aggs,
                group_by,
            };
        }

        if let Some(order) = &query.order_by {
            plan = PlanNode::Sort {
                input: Box::new(plan),
                field: order.field.clone(),
                desc: order.desc,
            };
        }

        if let Some(limit) = query.limit {
            plan = PlanNode::Limit {
                input: Box::new(plan),
                count: limit,
            };
        }

        plan
    }
}
