//! # 插件 trait 定义 — TSDB 可扩展性接口
//!
//! ## 设计目标
//!
//! 通过 trait 定义 TSDB 的三大扩展点，允许用户通过实现这些 trait
//! 来定制 TSDB 的行为，而无需修改核心代码：
//!
//! - **StoragePlugin**: 自定义数据持久化后端（S3、HDFS 等）
//! - **QueryPlugin**: 自定义查询处理逻辑
//! - **BusinessPlugin**: 业务规则校验和默认聚合配置
//!

use tsdb_types::model::DataPoint;

/// 存储插件 trait — 自定义数据持久化后端
///
/// 实现此 trait 可以将 TSDB 数据写入任意存储系统（如 S3、HDFS、自定义数据库等）。
pub trait StoragePlugin: Send + Sync {
    /// 返回插件唯一名称（用于注册和日志标识）
    fn name(&self) -> &str;
    
    /// 写入单个数据点到存储后端
    ///
    /// # 参数
    /// - `dp`: 待写入的数据点
    fn write(&self, dp: &DataPoint) -> Result<(), String>;
    
    /// 批量写入多个数据点（性能优化路径）
    ///
    /// # 参数
    /// - `dps`: 待批量写入的数据点列表
    fn write_batch(&self, dps: &[DataPoint]) -> Result<(), String> {
        for dp in dps { self.write(dp)?; }
        Ok(())
    }
}

/// 查询插件 trait — 自定义查询处理逻辑
///
/// 实现此 trait 可以扩展 TSDB 的查询能力（如支持新的查询语言、外部数据源 JOIN 等）。
pub trait QueryPlugin: Send + Sync {
    /// 返回插件唯一名称
    fn name(&self) -> &str;
    
    /// 执行自定义查询并返回 JSON 格式的结果
    ///
    /// # 参数
    /// - `query`: 查询字符串（格式由插件自行定义）
    fn query(&self, query: &str) -> Result<String, String>;
}

/// 业务插件 trait — 业务逻辑层面的扩展点
///
/// 实现此 trait 可以在数据写入前进行业务规则校验、
/// 自动补充默认聚合配置、或触发业务事件通知。
pub trait BusinessPlugin: Send + Sync {
    /// 返回插件唯一名称
    fn name(&self) -> &str;
    
    /// 校验数据点是否符合业务规则
    ///
    /// # 参数
    /// - `dp`: 待校验的数据点
    ///
    /// # 返回
    /// - `true`: 数据合法，允许写入
    /// - `false`: 数据不合规，拒绝写入
    fn validate(&self, dp: &DataPoint) -> bool;
    
    /// 返回该业务域推荐的默认聚合函数列表
    fn default_aggregations(&self) -> Vec<String>;
}
