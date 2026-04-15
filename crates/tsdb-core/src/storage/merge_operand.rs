use tsdb_types::model::FieldValue;

const FIELD_TYPE_FLOAT: u8 = 0x00;
const FIELD_TYPE_INTEGER: u8 = 0x01;
const FIELD_TYPE_STRING: u8 = 0x02;
const FIELD_TYPE_BOOLEAN: u8 = 0x03;

pub const MERGE_MAGIC: u16 = 0xFEED;

#[derive(Debug, Clone)]
pub struct MergedField {
    pub name: String,
    pub micro_offset: u32,
    pub value: FieldValue,
}

#[derive(Debug, Clone, Default)]
pub struct MergedBlock {
    pub fields: Vec<MergedField>,
}

impl MergedBlock {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + self.fields.len() * 20);
        buf.extend_from_slice(&MERGE_MAGIC.to_le_bytes());
        buf.extend_from_slice(&(self.fields.len() as u16).to_le_bytes());
        for f in &self.fields {
            encode_field_to_buf(&mut buf, &f.name, f.micro_offset, &f.value);
        }
        buf
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 4 { return None; }
        let magic = u16::from_le_bytes([data[0], data[1]]);
        if magic != MERGE_MAGIC { return None; }
        let field_count = u16::from_le_bytes([data[2], data[3]]) as usize;
        let mut fields = Vec::with_capacity(field_count);
        let mut offset = 4;
        for _ in 0..field_count {
            let (field, new_offset) = decode_field(data, offset)?;
            fields.push(field);
            offset = new_offset;
        }
        Some(Self { fields })
    }

    pub fn upsert_field(&mut self, new_field: MergedField) {
        for f in &mut self.fields {
            if f.name == new_field.name && f.micro_offset == new_field.micro_offset {
                f.value = new_field.value;
                return;
            }
        }
        self.fields.push(new_field);
    }

    pub fn to_data_points(
        &self,
        measurement: &str,
        block_start: i64,
        tags: tsdb_types::model::Tags,
    ) -> Vec<tsdb_types::model::DataPoint> {
        let mut offset_map: std::collections::BTreeMap<u32, std::collections::HashMap<String, FieldValue>> =
            std::collections::BTreeMap::new();
        for f in &self.fields {
            offset_map
                .entry(f.micro_offset)
                .or_default()
                .insert(f.name.clone(), f.value.clone());
        }

        offset_map
            .into_iter()
            .map(|(offset, fields)| {
                let ts = block_start + offset as i64;
                let mut dp = tsdb_types::model::DataPoint::new(measurement, ts);
                dp.tags = tags.clone();
                dp.fields = fields.into_iter().collect();
                dp
            })
            .collect()
    }

    pub fn get_data_point_at(
        &self,
        measurement: &str,
        block_start: i64,
        target_ts: i64,
        tags: tsdb_types::model::Tags,
    ) -> Option<tsdb_types::model::DataPoint> {
        let target_offset = (target_ts - block_start) as u32;
        let mut dp = tsdb_types::model::DataPoint::new(measurement, target_ts);
        dp.tags = tags;
        let mut found = false;
        for f in &self.fields {
            if f.micro_offset == target_offset {
                dp.fields.insert(f.name.clone(), f.value.clone());
                found = true;
            }
        }
        if found { Some(dp) } else { None }
    }
}

pub fn encode_merge_operand(field_name: &str, micro_offset: u32, value: &FieldValue) -> Vec<u8> {
    let mut buf = Vec::with_capacity(2 + field_name.len() + 4 + 9);
    encode_field_to_buf(&mut buf, field_name, micro_offset, value);
    buf
}

pub fn decode_merge_operand(data: &[u8]) -> Option<MergedField> {
    let (field, _) = decode_field(data, 0)?;
    Some(field)
}

fn encode_field_to_buf(buf: &mut Vec<u8>, name: &str, micro_offset: u32, value: &FieldValue) {
    match value {
        FieldValue::Float(f) => {
            buf.push(FIELD_TYPE_FLOAT);
            buf.push(name.len() as u8);
            buf.extend_from_slice(name.as_bytes());
            buf.extend_from_slice(&micro_offset.to_le_bytes());
            buf.extend_from_slice(&f.to_be_bytes());
        }
        FieldValue::Integer(i) => {
            buf.push(FIELD_TYPE_INTEGER);
            buf.push(name.len() as u8);
            buf.extend_from_slice(name.as_bytes());
            buf.extend_from_slice(&micro_offset.to_le_bytes());
            buf.extend_from_slice(&i.to_be_bytes());
        }
        FieldValue::String(s) => {
            buf.push(FIELD_TYPE_STRING);
            buf.push(name.len() as u8);
            buf.extend_from_slice(name.as_bytes());
            buf.extend_from_slice(&micro_offset.to_le_bytes());
            buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
            buf.extend_from_slice(s.as_bytes());
        }
        FieldValue::Boolean(b) => {
            buf.push(FIELD_TYPE_BOOLEAN);
            buf.push(name.len() as u8);
            buf.extend_from_slice(name.as_bytes());
            buf.extend_from_slice(&micro_offset.to_le_bytes());
            buf.push(if *b { 1 } else { 0 });
        }
    }
}

fn decode_field(data: &[u8], start: usize) -> Option<(MergedField, usize)> {
    if start >= data.len() { return None; }
    let field_type = data[start];
    let name_len = *data.get(start + 1)? as usize;
    let name_start = start + 2;
    let name_end = name_start + name_len;
    if name_end + 4 > data.len() { return None; }
    let name = String::from_utf8_lossy(&data[name_start..name_end]).to_string();
    let micro_offset = u32::from_le_bytes([
        data[name_end], data[name_end + 1], data[name_end + 2], data[name_end + 3],
    ]);
    let payload_start = name_end + 4;

    let (value, end) = match field_type {
        FIELD_TYPE_FLOAT => {
            if payload_start + 8 > data.len() { return None; }
            let f = f64::from_be_bytes(data[payload_start..payload_start + 8].try_into().ok()?);
            (FieldValue::Float(f), payload_start + 8)
        }
        FIELD_TYPE_INTEGER => {
            if payload_start + 8 > data.len() { return None; }
            let i = i64::from_be_bytes(data[payload_start..payload_start + 8].try_into().ok()?);
            (FieldValue::Integer(i), payload_start + 8)
        }
        FIELD_TYPE_STRING => {
            if payload_start + 4 > data.len() { return None; }
            let s_len = u32::from_le_bytes(data[payload_start..payload_start + 4].try_into().ok()?) as usize;
            let s_start = payload_start + 4;
            if s_start + s_len > data.len() { return None; }
            let s = String::from_utf8_lossy(&data[s_start..s_start + s_len]).to_string();
            (FieldValue::String(s), s_start + s_len)
        }
        FIELD_TYPE_BOOLEAN => {
            if payload_start >= data.len() { return None; }
            let b = data[payload_start] != 0;
            (FieldValue::Boolean(b), payload_start + 1)
        }
        _ => return None,
    };

    Some((MergedField { name, micro_offset, value }, end))
}

pub fn detect_value_format(data: &[u8]) -> ValueFormat {
    if data.len() >= 2 && u16::from_le_bytes([data[0], data[1]]) == MERGE_MAGIC {
        ValueFormat::Merged
    } else {
        ValueFormat::Raw
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValueFormat {
    Raw,
    Merged,
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_upsert_field_overwrite() {
        let mut block = MergedBlock::default();
        block.upsert_field(MergedField { name: "cpu".into(), micro_offset: 100, value: FieldValue::Float(0.5) });
        block.upsert_field(MergedField { name: "cpu".into(), micro_offset: 100, value: FieldValue::Float(0.9) });
        assert_eq!(block.fields.len(), 1);
        assert_eq!(block.fields[0].value, FieldValue::Float(0.9));
    }

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

    #[test]
    fn test_detect_value_format() {
        let merged = MergedBlock::default().encode();
        assert_eq!(detect_value_format(&merged), ValueFormat::Merged);
        let raw = vec![0x00, 0x01, 0x02, 0x03];
        assert_eq!(detect_value_format(&raw), ValueFormat::Raw);
    }
}
