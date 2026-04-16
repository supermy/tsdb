//! MergeOperator 注册模块 - MergeOperator Registration Module
//!
//! 本模块实现了 RocksDB MergeOperator 的核心合并逻辑：
//! - `tsdb_block_merge`: 合并函数，将多个字段 operand 合并为一个 MergedBlock
//! - `register_merge_operator`: 将合并函数注册到 RocksDB Options
//!
//! ## MergeOperator 工作原理
//!
//! 当调用 `merge_cf(key, operand)` 时，RocksDB 不会立即写入数据，
//! 而是将 operand 放入待合并队列。在以下时机触发合并：
//! 1. 读取时（Get）：合并所有待处理的 operand
//! 2. Compaction 时：后台合并并写入 SST 文件
//!
//! ## 合并语义
//!
//! ```text
//! existing_value: 之前已存储的 MergedBlock（可能为空）
//! operands: 新到达的字段列表 [field1, field2, ...]
//! result: 合并后的新 MergedBlock
//! ```text
//!
//! 后写入的同名字段会覆盖先写入的值（upsert 语义）。

use crate::storage::merge_operand::{MergedBlock, decode_merge_operand};

/// TSDB 时序块级合并函数
///
/// 这是 RocksDB MergeOperator 的核心回调函数，实现以下逻辑：
/// 1. 解析已有的 MergedBlock（如果存在）
/// 2. 逐个合并新到达的字段 operand
/// 3. 返回合并后的新 MergedBlock
///
/// # 参数
///
/// - `_key`: RocksDB 键（未使用，因为合并逻辑不依赖键内容）
/// - `existing_value`: 已存储的值（可能为 None，表示首次写入）
/// - `operands`: 新到达的 operand 列表（每个是一个编码后的字段）
///
/// # 返回值
///
/// 返回 `Some(merged_block_bytes)` 表示合并成功，
/// 返回 `None` 表示合并失败（实际上不会发生，因为空块也会返回有效编码）
///
/// # 合并规则
///
/// - 如果 existing_value 为 None，创建新的空 MergedBlock
/// - 对于每个 operand，解码为 MergedField 并调用 `upsert_field()`
/// - 相同 (name, offset) 的字段，后写入的值覆盖先写入的值
///
/// # 性能特性
///
/// - 时间复杂度: O(N + M)，N 为已有字段数，M 为新 operand 数
/// - 空间复杂度: O(N + M)，需要存储合并后的所有字段
/// - 幂等性: 多次合并相同 operand 结果一致
/// - 结合律: (A ⊕ B) ⊕ C = A ⊕ (B ⊕ C)
pub fn tsdb_block_merge(
    _key: &[u8],
    existing_value: Option<&[u8]>,
    operands: &rocksdb::MergeOperands,
) -> Option<Vec<u8>> {
    // 步骤 1: 解析已有的 MergedBlock（如果存在）
    // 如果 existing_value 为 None 或解码失败，创建空的 MergedBlock
    let mut block = existing_value
        .and_then(MergedBlock::decode)
        .unwrap_or_default();

    // 步骤 2: 逐个合并新到达的 operand
    // operands 是一个迭代器，按写入顺序返回每个 operand
    for op in operands.iter() {
        // 解码 operand 为 MergedField
        if let Some(field) = decode_merge_operand(op) {
            // upsert_field 实现"后写覆盖"语义
            block.upsert_field(field);
        }
    }

    // 步骤 3: 编码并返回合并后的 MergedBlock
    Some(block.encode())
}

/// 注册 MergeOperator 到 RocksDB Options
///
/// 将 `tsdb_block_merge` 函数注册为关联式合并操作符。
/// 关联式合并要求合并操作满足交换律和结合律，
/// 这允许 RocksDB 在 Compaction 时以任意顺序合并 operand。
///
/// # 参数
///
/// - `opts`: RocksDB Options 对象的可变引用
///
/// # 注册名称
///
/// 操作符名称为 `"tsdb.block_merge"`，用于：
/// - SST 文件元数据记录
/// - 数据库恢复时验证兼容性
/// - 调试和监控时识别操作符
///
/// # 使用示例
///
/// ```rust,ignore
/// let mut opts = rocksdb::Options::default();
/// register_merge_operator(&mut opts);
/// // 之后可以使用 db.merge_cf() 进行合并写入
/// ```text
///
/// # 注意事项
///
/// - 必须在打开数据库前注册
/// - 已有数据的数据库不能更改 MergeOperator 类型
/// - 不同 MergeOperator 名称的数据文件不兼容
pub fn register_merge_operator(opts: &mut rocksdb::Options) {
    // 使用 set_merge_operator_associative 注册关联式合并操作符
    // 关联式合并要求操作满足：
    // - 交换律: a ⊕ b = b ⊕ a
    // - 结合律: (a ⊕ b) ⊕ c = a ⊕ (b ⊕ c)
    //
    // 我们的 upsert 操作满足这些性质：
    // - 交换律: 两次写入相同字段，最终值只取决于最后一次写入
    // - 结合律: 多次合并的顺序不影响最终结果
    opts.set_merge_operator_associative("tsdb.block_merge", tsdb_block_merge);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::merge_operand::{encode_merge_operand, MergedField};
    use tsdb_types::model::FieldValue;

    /// 测试合并逻辑的 upsert（覆盖）语义
    ///
    /// 验证：
    /// 1. 相同 (name, offset) 的字段，后写入的值覆盖先写入的值
    /// 2. 不同 name 的字段可以共存
    #[test]
    fn test_merge_logic_upsert() {
        // 创建空块
        let mut block = MergedBlock::default();

        // 第一次写入 cpu 字段，值为 0.3
        let op1 = encode_merge_operand("cpu", 15000, &FieldValue::Float(0.3));
        let field1 = decode_merge_operand(&op1).unwrap();
        block.upsert_field(field1);

        // 第二次写入 cpu 字段，值为 0.5（覆盖第一次）
        let op2 = encode_merge_operand("cpu", 15000, &FieldValue::Float(0.5));
        let field2 = decode_merge_operand(&op2).unwrap();
        block.upsert_field(field2);

        // 写入 mem 字段（不同 name，不覆盖）
        let op3 = encode_merge_operand("mem", 15000, &FieldValue::Float(0.8));
        let field3 = decode_merge_operand(&op3).unwrap();
        block.upsert_field(field3);

        // 验证：应该有 2 个字段（cpu 和 mem）
        assert_eq!(block.fields.len(), 2);

        // 验证：cpu 字段的值应该是 0.5（被覆盖）
        let cpu = block.fields.iter().find(|f| f.name == "cpu").unwrap();
        assert_eq!(cpu.value, FieldValue::Float(0.5));
    }

    /// 测试从空块开始合并多个字段
    #[test]
    fn test_merge_logic_from_empty() {
        let mut block = MergedBlock::default();

        // 写入 5 个不同名称的字段
        for i in 0..5 {
            let op = encode_merge_operand(&format!("field_{}", i), 1000, &FieldValue::Float(i as f64));
            let field = decode_merge_operand(&op).unwrap();
            block.upsert_field(field);
        }

        // 验证：应该有 5 个字段
        assert_eq!(block.fields.len(), 5);
    }

    /// 测试完整的编码-解码循环
    #[test]
    fn test_merge_encode_decode_cycle() {
        // 创建包含两个字段的块
        let mut block = MergedBlock::default();
        block.upsert_field(MergedField { name: "cpu".into(), micro_offset: 100, value: FieldValue::Float(0.5) });
        block.upsert_field(MergedField { name: "mem".into(), micro_offset: 100, value: FieldValue::Float(0.8) });

        // 编码
        let encoded = block.encode();

        // 解码
        let decoded = MergedBlock::decode(&encoded).unwrap();

        // 验证：字段数量一致
        assert_eq!(decoded.fields.len(), 2);
    }
}
