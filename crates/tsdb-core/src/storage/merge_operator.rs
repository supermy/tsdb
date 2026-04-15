use crate::storage::merge_operand::{MergedBlock, decode_merge_operand, MergedField};

pub fn tsdb_block_merge(
    _key: &[u8],
    existing_value: Option<&[u8]>,
    operands: &rocksdb::MergeOperands,
) -> Option<Vec<u8>> {
    let mut block = existing_value
        .and_then(MergedBlock::decode)
        .unwrap_or_default();

    for op in operands.iter() {
        if let Some(field) = decode_merge_operand(op) {
            block.upsert_field(field);
        }
    }

    Some(block.encode())
}

pub fn register_merge_operator(opts: &mut rocksdb::Options) {
    opts.set_merge_operator_associative("tsdb.block_merge", tsdb_block_merge);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::merge_operand::encode_merge_operand;
    use tsdb_types::model::FieldValue;

    #[test]
    fn test_merge_logic_upsert() {
        let mut block = MergedBlock::default();
        let op1 = encode_merge_operand("cpu", 15000, &FieldValue::Float(0.3));
        let field1 = decode_merge_operand(&op1).unwrap();
        block.upsert_field(field1);

        let op2 = encode_merge_operand("cpu", 15000, &FieldValue::Float(0.5));
        let field2 = decode_merge_operand(&op2).unwrap();
        block.upsert_field(field2);

        let op3 = encode_merge_operand("mem", 15000, &FieldValue::Float(0.8));
        let field3 = decode_merge_operand(&op3).unwrap();
        block.upsert_field(field3);

        assert_eq!(block.fields.len(), 2);
        let cpu = block.fields.iter().find(|f| f.name == "cpu").unwrap();
        assert_eq!(cpu.value, FieldValue::Float(0.5));
    }

    #[test]
    fn test_merge_logic_from_empty() {
        let mut block = MergedBlock::default();
        for i in 0..5 {
            let op = encode_merge_operand(&format!("field_{}", i), 1000, &FieldValue::Float(i as f64));
            let field = decode_merge_operand(&op).unwrap();
            block.upsert_field(field);
        }
        assert_eq!(block.fields.len(), 5);
    }

    #[test]
    fn test_merge_encode_decode_cycle() {
        let mut block = MergedBlock::default();
        block.upsert_field(MergedField { name: "cpu".into(), micro_offset: 100, value: FieldValue::Float(0.5) });
        block.upsert_field(MergedField { name: "mem".into(), micro_offset: 100, value: FieldValue::Float(0.8) });

        let encoded = block.encode();
        let decoded = MergedBlock::decode(&encoded).unwrap();
        assert_eq!(decoded.fields.len(), 2);
    }
}
