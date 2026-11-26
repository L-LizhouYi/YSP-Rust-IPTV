//! 极简 protobuf 读取器（只读，零拷贝）。

use crate::error::YspError;

pub const WIRE_VARINT: u8 = 0;
pub const WIRE_64BIT: u8 = 1;
pub const WIRE_LEN: u8 = 2;
pub const WIRE_32BIT: u8 = 5;

#[derive(Debug, Clone, Copy)]
pub enum Value<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
}

#[derive(Debug, Clone, Copy)]
pub struct Field<'a> {
    pub number: u32,
    pub wire: u8,
    pub value: Value<'a>,
}

impl<'a> Field<'a> {
    pub fn as_bytes(&self) -> &'a [u8] {
        match self.value {
            Value::Bytes(b) => b,
            _ => &[],
        }
    }

    pub fn as_text(&self) -> String {
        String::from_utf8_lossy(self.as_bytes()).into_owned()
    }
}

fn read_varint(buf: &[u8], pos: &mut usize) -> Result<u64, YspError> {
    let mut shift = 0u32;
    let mut value = 0u64;
    loop {
        let byte = *buf
            .get(*pos)
            .ok_or_else(|| YspError::Protocol("protobuf 截断".into()))?;
        *pos += 1;
        value |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift >= 64 {
            return Err(YspError::Protocol("protobuf varint 溢出".into()));
        }
    }
}

pub fn parse(buf: &[u8]) -> Result<Vec<Field<'_>>, YspError> {
    let mut fields = Vec::new();
    let mut pos = 0usize;
    while pos < buf.len() {
        let key = read_varint(buf, &mut pos)?;
        let number = (key >> 3) as u32;
        let wire = (key & 7) as u8;
        let value = match wire {
            WIRE_VARINT => Value::Varint(read_varint(buf, &mut pos)?),
            WIRE_64BIT => {
                let end = pos + 8;
                let slice = buf
                    .get(pos..end)
                    .ok_or_else(|| YspError::Protocol("protobuf 截断".into()))?;
                pos = end;
                Value::Bytes(slice)
            }
            WIRE_LEN => {
                let len = read_varint(buf, &mut pos)? as usize;
                let end = pos
                    .checked_add(len)
                    .ok_or_else(|| YspError::Protocol("protobuf 长度溢出".into()))?;
                let slice = buf
                    .get(pos..end)
                    .ok_or_else(|| YspError::Protocol("protobuf 截断".into()))?;
                pos = end;
                Value::Bytes(slice)
            }
            WIRE_32BIT => {
                let end = pos + 4;
                let slice = buf
                    .get(pos..end)
                    .ok_or_else(|| YspError::Protocol("protobuf 截断".into()))?;
                pos = end;
                Value::Bytes(slice)
            }
            other => {
                return Err(YspError::Protocol(format!("未知 wire type {other}")));
            }
        };
        fields.push(Field { number, wire, value });
    }
    Ok(fields)
}

/// 深度优先遍历所有「看起来能解析」的子消息（带深度上限防止恶意数据爆栈）。
pub fn walk<'a>(buf: &'a [u8], f: &mut impl FnMut(&[Field<'a>])) {
    walk_depth(buf, 0, f);
}

fn walk_depth<'a>(buf: &'a [u8], depth: usize, f: &mut impl FnMut(&[Field<'a>])) {
    if depth > 16 {
        return;
    }
    let Ok(fields) = parse(buf) else {
        return;
    };
    f(&fields);
    for field in &fields {
        if field.wire == WIRE_LEN {
            if let Value::Bytes(sub) = field.value {
                if sub.len() > 1 {
                    walk_depth(sub, depth + 1, f);
                }
            }
        }
    }
}

pub fn text_field(fields: &[Field<'_>], number: u32) -> Option<String> {
    fields
        .iter()
        .find(|f| f.number == number && f.wire == WIRE_LEN)
        .map(Field::as_text)
}

pub fn varint_field(fields: &[Field<'_>], number: u32, default: u64) -> u64 {
    fields
        .iter()
        .find(|f| f.number == number && f.wire == WIRE_VARINT)
        .and_then(|f| match f.value {
            Value::Varint(v) => Some(v),
            _ => None,
        })
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            out.push(byte | if value != 0 { 0x80 } else { 0 });
            if value == 0 {
                return out;
            }
        }
    }

    fn f_varint(number: u32, value: u64) -> Vec<u8> {
        let mut out = varint(((number as u64) << 3) | 0);
        out.extend(varint(value));
        out
    }

    fn f_bytes(number: u32, data: &[u8]) -> Vec<u8> {
        let mut out = varint(((number as u64) << 3) | 2);
        out.extend(varint(data.len() as u64));
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn parse_roundtrip() {
        let mut buf = f_varint(1, 300);
        buf.extend(f_bytes(2, b"hello"));
        buf.extend(f_bytes(3, &[1, 2]));
        let fields = parse(&buf).unwrap();
        assert_eq!(fields[0].number, 1);
        assert!(matches!(fields[0].value, Value::Varint(300)));
        assert_eq!(fields[1].as_text(), "hello");
        assert_eq!(fields[2].as_bytes(), &[1, 2]);
    }
}
