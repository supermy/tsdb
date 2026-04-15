//! 合并操作数模块 - MergeOperand Module
//!
//! 本模块实现了 RocksDB MergeOperator 的核心数据结构：
//! - `MergedBlock`: 合并后的数据块，包含多个字段
//! - `MergedField`: 单个字段，包含字段名、微秒偏移量和字段值
//! - 编解码函数：将字段编码为二进制格式，用于 RocksDB merge 操作
//!
//! ## 核心优化原理
//!
//! 传统方案：每个字段一个 KV 对，查询时需要多次 I/O
//! ```text
//! Key: "cpu|hash|block_ts\x00usage:offset" → Value: [0x00][f64]
//! Key: "cpu|hash|block_ts\x00mem:offset"  → Value: [0x00][f64]
//! ```text
//!
//! MergeOperator 方案：同一 RowKey 的所有字段合并为一个 MergedBlock
//! ```text
//! Key: "cpu|hash|block_ts" → Value: [0xFEED][field_count][field1][field2]...
//! ```text
//!
//! 性能提升：单点查询从 N 次 I/O 降为 1 次 I/O

use tsdb_types::model::FieldValue;

/// 字段类型标识符 - Field Type Identifiers
/// 用于二进制编码时标识字段的数据类型
const FIELD_TYPE_FLOAT: u8 = 0x00;    // 浮点数类型
const FIELD_TYPE_INTEGER: u8 = 0x01;  // 整数类型
const FIELD_TYPE_STRING: u8 = 0x02;   // 字符串类型
const FIELD_TYPE_BOOLEAN: u8 = 0x03;  // 布尔类型

/// MergedBlock 魔数 - Magic Number for MergedBlock
/// 用于在读取时快速识别数据格式是否为合并格式
/// 值为 0xFEED，便于调试和格式检测
pub const MERGE_MAGIC: u16 = 0xFEED;

/// 合并字段 - Merged Field
///
/// 表示 MergedBlock 中的单个字段，包含：
/// - `name`: 字段名称（如 "cpu_usage", "memory"）
/// - `micro_offset`: 相对于块起始时间戳的微秒偏移量
/// - `value`: 字段值（浮点数、整数、字符串或布尔值）
///
/// # 示例
///
/// ```text
/// let field = MergedField {
///     name: "cpu".to_string(),
///     micro_offset: 15000,  // 块起始后 15 毫秒
///     value: FieldValue::Float(0.75),
/// };
/// ```text
#[derive(Debug, Clone)]
pub struct MergedField {
    /// 字段名称
    pub name: String,
    /// 微秒偏移量（相对于块起始时间戳）
    pub micro_offset: u32,
    /// 字段值
    pub value: FieldValue,
}

/// 合并数据块 - Merged Block
///
/// RocksDB MergeOperator 合并后的数据块，包含同一 RowKey 下的所有字段。
/// 这是实现"N次访问合并为1次"的核心数据结构。
///
/// ## 二进制格式
///
/// ```text
/// ┌────────┬────────────┬─────────────────────────────────────┐
/// │ Magic  │ FieldCount │ Fields[]                            │
/// │ (2B)   │ (u16 LE)   │ (连续排列，每个字段变长)              │
/// ├────────┼────────────┼─────────────────────────────────────┤
/// │ 0xFEED │ N          │ [Field_1][Field_2]...[Field_N]      │
/// └────────┴────────────┴─────────────────────────────────────┘
/// ```text
///
/// ## 使用场景
///
/// 1. 写入时：通过 `upsert_field()` 逐个添加字段
/// 2. 读取时：通过 `decode()` 从二进制解析
/// 3. 查询时：通过 `get_data_point_at()` 获取特定时间点的数据
#[derive(Debug, Clone, Default)]
pub struct MergedBlock {
    /// 字段列表
    pub fields: Vec<MergedField>,
}

impl MergedBlock {
    /// 编码为二进制格式
    ///
    /// 将 MergedBlock 编码为紧凑的二进制格式，用于存储到 RocksDB。
    ///
    /// # 格式
    ///
    /// ```text
    /// [magic:2B][field_count:2B][field1][field2]...[fieldN]
    /// ```text
    ///
    /// # 返回值
    ///
    /// 编码后的二进制数据
    pub fn encode(&self) -> Vec<u8> {
        // 预分配容量：4字节头部 + 每个字段约20字节
        let mut buf = Vec::with_capacity(4 + self.fields.len() * 20);

        // 写入魔数（小端序）
        buf.extend_from_slice(&MERGE_MAGIC.to_le_bytes());

        // 写入字段数量（小端序，最多支持 65535 个字段）
        buf.extend_from_slice(&(self.fields.len() as u16).to_le_bytes());

        // 逐个编码字段
        for f in &self.fields {
            encode_field_to_buf(&mut buf, &f.name, f.micro_offset, &f.value);
        }

        buf
    }

    /// 从二进制格式解码
    ///
    /// 从 RocksDB 读取的二进制数据解析为 MergedBlock。
    ///
    /// # 参数
    ///
    /// - `data`: 二进制数据
    ///
    /// # 返回值
    ///
    /// 解析成功返回 `Some(MergedBlock)`，格式错误返回 `None`
    ///
    /// # 错误情况
    ///
    /// - 数据长度不足 4 字节
    /// - 魔数不匹配（不是 0xFEED）
    /// - 字段解码失败
    pub fn decode(data: &[u8]) -> Option<Self> {
        // 最小长度检查：魔数(2B) + 字段数(2B) = 4 字节
        if data.len() < 4 { return None; }

        // 验证魔数
        let magic = u16::from_le_bytes([data[0], data[1]]);
        if magic != MERGE_MAGIC { return None; }

        // 读取字段数量
        let field_count = u16::from_le_bytes([data[2], data[3]]) as usize;

        // 预分配字段向量
        let mut fields = Vec::with_capacity(field_count);
        let mut offset = 4;  // 跳过头部

        // 逐个解码字段
        for _ in 0..field_count {
            let (field, new_offset) = decode_field(data, offset)?;
            fields.push(field);
            offset = new_offset;
        }

        Some(Self { fields })
    }

    /// 插入或更新字段
    ///
    /// 将新字段添加到块中。如果已存在相同名称和偏移量的字段，则更新其值。
    /// 这实现了 MergeOperator 的"后写覆盖"语义。
    ///
    /// # 参数
    ///
    /// - `new_field`: 要插入/更新的字段
    ///
    /// # 示例
    ///
    /// ```text
    /// let mut block = MergedBlock::default();
    /// block.upsert_field(MergedField { name: "cpu".into(), micro_offset: 100, value: FieldValue::Float(0.5) });
    /// block.upsert_field(MergedField { name: "cpu".into(), micro_offset: 100, value: FieldValue::Float(0.9) });
    /// // 结果：只有一个字段，值为 0.9
    /// ```text
    pub fn upsert_field(&mut self, new_field: MergedField) {
        // 查找是否已存在相同 (name, offset) 的字段
        for f in &mut self.fields {
            if f.name == new_field.name && f.micro_offset == new_field.micro_offset {
                // 找到匹配，更新值
                f.value = new_field.value;
                return;
            }
        }
        // 未找到匹配，添加新字段
        self.fields.push(new_field);
    }

    /// 转换为数据点列表
    ///
    /// 将 MergedBlock 中的所有字段按时间戳分组，转换为多个 DataPoint。
    /// 用于范围查询时返回完整的数据点。
    ///
    /// # 参数
    ///
    /// - `measurement`: 指标名称
    /// - `block_start`: 块起始时间戳（微秒）
    /// - `tags`: 标签集合
    ///
    /// # 返回值
    ///
    /// 按时间戳排序的数据点列表
    ///
    /// # 算法
    ///
    /// 1. 按 micro_offset 分组字段
    /// 2. 计算每个分组的绝对时间戳
    /// 3. 构造 DataPoint
    pub fn to_data_points(
        &self,
        measurement: &str,
        block_start: i64,
        tags: tsdb_types::model::Tags,
    ) -> Vec<tsdb_types::model::DataPoint> {
        // 使用 BTreeMap 按 offset 排序
        let mut offset_map: std::collections::BTreeMap<u32, std::collections::HashMap<String, FieldValue>> =
            std::collections::BTreeMap::new();

        // 按 offset 分组字段
        for f in &self.fields {
            offset_map
                .entry(f.micro_offset)
                .or_default()
                .insert(f.name.clone(), f.value.clone());
        }

        // 转换为 DataPoint
        offset_map
            .into_iter()
            .map(|(offset, fields)| {
                let ts = block_start + offset as i64;  // 计算绝对时间戳
                let mut dp = tsdb_types::model::DataPoint::new(measurement, ts);
                dp.tags = tags.clone();
                dp.fields = fields.into_iter().collect();
                dp
            })
            .collect()
    }

    /// 获取特定时间点的数据点
    ///
    /// 从 MergedBlock 中提取指定时间戳的所有字段，构造单个 DataPoint。
    /// 这是单点查询的核心方法，实现"1次 get 获取完整数据点"。
    ///
    /// # 参数
    ///
    /// - `measurement`: 指标名称
    /// - `block_start`: 块起始时间戳（微秒）
    /// - `target_ts`: 目标时间戳（微秒）
    /// - `tags`: 标签集合
    ///
    /// # 返回值
    ///
    /// 如果找到匹配的字段，返回 `Some(DataPoint)`，否则返回 `None`
    pub fn get_data_point_at(
        &self,
        measurement: &str,
        block_start: i64,
        target_ts: i64,
        tags: tsdb_types::model::Tags,
    ) -> Option<tsdb_types::model::DataPoint> {
        // 计算目标时间戳对应的微秒偏移量
        let target_offset = (target_ts - block_start) as u32;

        let mut dp = tsdb_types::model::DataPoint::new(measurement, target_ts);
        dp.tags = tags;
        let mut found = false;

        // 收集所有匹配该偏移量的字段
        for f in &self.fields {
            if f.micro_offset == target_offset {
                dp.fields.insert(f.name.clone(), f.value.clone());
                found = true;
            }
        }

        if found { Some(dp) } else { None }
    }
}

/// 编码合并操作数
///
/// 将单个字段编码为 MergeOperand 格式，用于 RocksDB merge 操作。
/// 这是 `merge_cf()` 调用时传递的 value 参数。
///
/// # 参数
///
/// - `field_name`: 字段名称
/// - `micro_offset`: 微秒偏移量
/// - `value`: 字段值
///
/// # 返回值
///
/// 编码后的二进制数据
///
/// # 格式
///
/// ```text
/// ┌──────┬─────────┬──────────┬──────────┬───────────────┐
/// │ Type │ NameLen │ Name     │ Offset   │ Payload       │
/// │ (1B) │ (1B)    │ (Var)    │ (4B LE)  │ (Var)         │
/// └──────┴─────────┴──────────┴──────────┴───────────────┘
/// ```text
pub fn encode_merge_operand(field_name: &str, micro_offset: u32, value: &FieldValue) -> Vec<u8> {
    // 预分配容量：类型(1) + 名称长度(1) + 名称 + 偏移量(4) + 值(最多9)
    let mut buf = Vec::with_capacity(2 + field_name.len() + 4 + 9);
    encode_field_to_buf(&mut buf, field_name, micro_offset, value);
    buf
}

/// 解码合并操作数
///
/// 从二进制数据解析单个字段。
///
/// # 参数
///
/// - `data`: 二进制数据
///
/// # 返回值
///
/// 解析成功返回 `Some(MergedField)`，失败返回 `None`
pub fn decode_merge_operand(data: &[u8]) -> Option<MergedField> {
    let (field, _) = decode_field(data, 0)?;
    Some(field)
}

/// 编码字段到缓冲区
///
/// 内部函数，将单个字段编码并追加到缓冲区。
///
/// # 参数
///
/// - `buf`: 目标缓冲区
/// - `name`: 字段名称
/// - `micro_offset`: 微秒偏移量
/// - `value`: 字段值
fn encode_field_to_buf(buf: &mut Vec<u8>, name: &str, micro_offset: u32, value: &FieldValue) {
    match value {
        // 浮点数编码：类型(1) + 名称长度(1) + 名称 + 偏移量(4) + f64(8)
        FieldValue::Float(f) => {
            buf.push(FIELD_TYPE_FLOAT);
            buf.push(name.len() as u8);
            buf.extend_from_slice(name.as_bytes());
            buf.extend_from_slice(&micro_offset.to_le_bytes());
            buf.extend_from_slice(&f.to_be_bytes());  // 大端序保持精度
        }
        // 整数编码：类型(1) + 名称长度(1) + 名称 + 偏移量(4) + i64(8)
        FieldValue::Integer(i) => {
            buf.push(FIELD_TYPE_INTEGER);
            buf.push(name.len() as u8);
            buf.extend_from_slice(name.as_bytes());
            buf.extend_from_slice(&micro_offset.to_le_bytes());
            buf.extend_from_slice(&i.to_be_bytes());
        }
        // 字符串编码：类型(1) + 名称长度(1) + 名称 + 偏移量(4) + 长度(4) + 内容
        FieldValue::String(s) => {
            buf.push(FIELD_TYPE_STRING);
            buf.push(name.len() as u8);
            buf.extend_from_slice(name.as_bytes());
            buf.extend_from_slice(&micro_offset.to_le_bytes());
            buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
            buf.extend_from_slice(s.as_bytes());
        }
        // 布尔编码：类型(1) + 名称长度(1) + 名称 + 偏移量(4) + 值(1)
        FieldValue::Boolean(b) => {
            buf.push(FIELD_TYPE_BOOLEAN);
            buf.push(name.len() as u8);
            buf.extend_from_slice(name.as_bytes());
            buf.extend_from_slice(&micro_offset.to_le_bytes());
            buf.push(if *b { 1 } else { 0 });
        }
    }
}

/// 解码单个字段
///
/// 内部函数，从二进制数据的指定位置解析单个字段。
///
/// # 参数
///
/// - `data`: 二进制数据
/// - `start`: 起始位置
///
/// # 返回值
///
/// 解析成功返回 `Some((MergedField, end_position))`，失败返回 `None`
fn decode_field(data: &[u8], start: usize) -> Option<(MergedField, usize)> {
    // 边界检查
    if start >= data.len() { return None; }

    // 读取字段类型
    let field_type = data[start];

    // 读取名称长度
    let name_len = *data.get(start + 1)? as usize;
    let name_start = start + 2;
    let name_end = name_start + name_len;

    // 边界检查
    if name_end + 4 > data.len() { return None; }

    // 读取字段名称
    let name = String::from_utf8_lossy(&data[name_start..name_end]).to_string();

    // 读取微秒偏移量（小端序）
    let micro_offset = u32::from_le_bytes([
        data[name_end], data[name_end + 1], data[name_end + 2], data[name_end + 3],
    ]);

    let payload_start = name_end + 4;

    // 根据字段类型解析值
    let (value, end) = match field_type {
        // 浮点数：8 字节大端序
        FIELD_TYPE_FLOAT => {
            if payload_start + 8 > data.len() { return None; }
            let f = f64::from_be_bytes(data[payload_start..payload_start + 8].try_into().ok()?);
            (FieldValue::Float(f), payload_start + 8)
        }
        // 整数：8 字节大端序
        FIELD_TYPE_INTEGER => {
            if payload_start + 8 > data.len() { return None; }
            let i = i64::from_be_bytes(data[payload_start..payload_start + 8].try_into().ok()?);
            (FieldValue::Integer(i), payload_start + 8)
        }
        // 字符串：4 字节长度 + 内容
        FIELD_TYPE_STRING => {
            if payload_start + 4 > data.len() { return None; }
            let s_len = u32::from_le_bytes(data[payload_start..payload_start + 4].try_into().ok()?) as usize;
            let s_start = payload_start + 4;
            if s_start + s_len > data.len() { return None; }
            let s = String::from_utf8_lossy(&data[s_start..s_start + s_len]).to_string();
            (FieldValue::String(s), s_start + s_len)
        }
        // 布尔：1 字节
        FIELD_TYPE_BOOLEAN => {
            if payload_start >= data.len() { return None; }
            let b = data[payload_start] != 0;
            (FieldValue::Boolean(b), payload_start + 1)
        }
        // 未知类型
        _ => return None,
    };

    Some((MergedField { name, micro_offset, value }, end))
}

/// 检测值格式
///
/// 快速检测二进制数据是 MergedBlock 格式还是原始格式。
/// 用于读取路径的向后兼容处理。
///
/// # 参数
///
/// - `data`: 二进制数据
///
/// # 返回值
///
/// - `ValueFormat::Merged`: MergedBlock 格式（魔数为 0xFEED）
/// - `ValueFormat::Raw`: 原始格式（旧版单字段格式）
pub fn detect_value_format(data: &[u8]) -> ValueFormat {
    if data.len() >= 2 && u16::from_le_bytes([data[0], data[1]]) == MERGE_MAGIC {
        ValueFormat::Merged
    } else {
        ValueFormat::Raw
    }
}

/// 值格式枚举
///
/// 表示存储在 RocksDB 中的值的数据格式。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValueFormat {
    /// 原始格式（旧版，每个字段一个 KV）
    Raw,
    /// 合并格式（新版，使用 MergeOperator）
    Merged,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试合并操作数的编解码往返
    #[test]
    fn test_merge_operand_roundtrip() {
        let cases = vec![
            ("usage", 15000u32, FieldValue::Float(0.75)),
            ("count", 15000u32, FieldValue::Integer(42)),
            ("host", 15000u32, FieldValue::String("server01".to_string())),
            ("active", 15000u32, FieldValue::Boolean(true)),
        ];
        for (name, offset, value) in cases {
            let encoded = encode_merge_operand(name, offset, &value);
            let decoded = decode_merge_operand(&encoded).unwrap();
            assert_eq!(decoded.name, name);
            assert_eq!(decoded.micro_offset, offset);
            assert_eq!(decoded.value, value);
        }
    }

    /// 测试 MergedBlock 的编解码往返
    #[test]
    fn test_merged_block_roundtrip() {
        let block = MergedBlock {
            fields: vec![
                MergedField { name: "cpu".into(), micro_offset: 15000, value: FieldValue::Float(0.5) },
                MergedField { name: "mem".into(), micro_offset: 15000, value: FieldValue::Float(0.8) },
                MergedField { name: "count".into(), micro_offset: 15000, value: FieldValue::Integer(100) },
            ],
        };
        let encoded = block.encode();
        let decoded = MergedBlock::decode(&encoded).unwrap();
        assert_eq!(decoded.fields.len(), 3);
        assert_eq!(decoded.fields[0].name, "cpu");
        assert_eq!(decoded.fields[1].name, "mem");
        assert_eq!(decoded.fields[2].name, "count");
    }

    /// 测试 upsert_field 的覆盖语义
    #[test]
    fn test_upsert_field_overwrite() {
        let mut block = MergedBlock::default();
        block.upsert_field(MergedField { name: "cpu".into(), micro_offset: 100, value: FieldValue::Float(0.5) });
        block.upsert_field(MergedField { name: "cpu".into(), micro_offset: 100, value: FieldValue::Float(0.9) });
        assert_eq!(block.fields.len(), 1);
        assert_eq!(block.fields[0].value, FieldValue::Float(0.9));
    }

    /// 测试转换为数据点列表
    #[test]
    fn test_to_data_points() {
        let block = MergedBlock {
            fields: vec![
                MergedField { name: "cpu".into(), micro_offset: 10000, value: FieldValue::Float(0.5) },
                MergedField { name: "mem".into(), micro_offset: 10000, value: FieldValue::Float(0.8) },
                MergedField { name: "cpu".into(), micro_offset: 20000, value: FieldValue::Float(0.6) },
            ],
        };
        let mut tags = std::collections::BTreeMap::new();
        tags.insert("host".to_string(), "s1".to_string());
        let dps = block.to_data_points("cpu", 1000000, tags);
        assert_eq!(dps.len(), 2);
        assert_eq!(dps[0].timestamp, 1010000);
        assert_eq!(dps[0].fields.len(), 2);
        assert_eq!(dps[1].timestamp, 1020000);
        assert_eq!(dps[1].fields.len(), 1);
    }

    /// 测试格式检测
    #[test]
    fn test_detect_value_format() {
        let merged = MergedBlock::default().encode();
        assert_eq!(detect_value_format(&merged), ValueFormat::Merged);
        let raw = vec![0x00, 0x01, 0x02, 0x03];
        assert_eq!(detect_value_format(&raw), ValueFormat::Raw);
    }
}
