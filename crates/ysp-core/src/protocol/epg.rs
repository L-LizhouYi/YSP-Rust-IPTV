//! EPG（节目单）解析。
//!
//! GET https://capi.yangshipin.cn/api/yspepg/program/{livepid}/{YYYYMMDD}
//! 每条节目：#1 program_id / #2 名称 / #3 start_ts / #4 end_ts / #5 "00:00" / #6 "00:30"

use crate::{
    models::Program,
    protocol::protobuf::{parse, text_field, varint_field, Value, WIRE_LEN},
};

pub fn parse_epg(buf: &[u8]) -> Vec<Program> {
    let mut programs = Vec::new();
    let Ok(fields) = parse(buf) else { return programs };
    for entry in fields {
        if entry.wire != WIRE_LEN {
            continue;
        }
        let Value::Bytes(body) = entry.value else { continue };
        let Ok(inner) = parse(body) else { continue };
        let Some(name) = text_field(&inner, 2) else { continue };
        let start = varint_field(&inner, 3, 0) as i64;
        if name.is_empty() || start == 0 {
            continue;
        }
        programs.push(Program {
            program_id: varint_field(&inner, 1, 0).to_string(),
            name,
            start_ts: start,
            end_ts: varint_field(&inner, 4, 0) as i64,
            start_time: text_field(&inner, 5).unwrap_or_default(),
            end_time: text_field(&inner, 6).unwrap_or_default(),
        });
    }
    programs.sort_by_key(|p| p.start_ts);
    programs
}

pub fn current_program(programs: &[Program], at: i64) -> Option<&Program> {
    programs
        .iter()
        .find(|p| p.start_ts <= at && at < p.end_ts)
        .or_else(|| programs.first())
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
    fn f_text(number: u32, text: &str) -> Vec<u8> {
        f_bytes(number, text.as_bytes())
    }

    fn program(pid: u64, name: &str, start: u64, end: u64, a: &str, b: &str) -> Vec<u8> {
        let mut body = f_varint(1, pid);
        body.extend(f_text(2, name));
        body.extend(f_varint(3, start));
        body.extend(f_varint(4, end));
        body.extend(f_text(5, a));
        body.extend(f_text(6, b));
        f_bytes(1, &body)
    }

    #[test]
    fn parse_and_sort() {
        let mut buf = program(1, "午夜新闻", 1000, 2000, "00:00", "00:30");
        buf.extend(program(2, "高端访谈", 2000, 2500, "00:30", "01:00"));
        let programs = parse_epg(&buf);
        assert_eq!(programs.len(), 2);
        assert_eq!(programs[0].name, "午夜新闻");
        assert_eq!(programs[0].duration(), 1000);
        assert_eq!(programs[1].start_time, "00:30");
        assert_eq!(current_program(&programs, 2100).unwrap().name, "高端访谈");
    }
}
