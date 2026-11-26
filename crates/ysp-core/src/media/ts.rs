//! MPEG-TS 基础解析：PAT/PMT -> 首个 H.264 视频 PID。

use crate::config::TS_PACKET;

pub const STREAM_TYPE_H264: u8 = 0x1B;
pub const STREAM_TYPE_HEVC: u8 = 0x24;

/// 遍历指定 PID 上的 payload，返回 (packet_off, pusi, payload_start)。
fn payload_spans(data: &[u8], pid: u16) -> Vec<(usize, bool, usize)> {
    let mut out = Vec::new();
    let packets = data.len() / TS_PACKET;
    for i in 0..packets {
        let off = i * TS_PACKET;
        if data[off] != 0x47 {
            continue;
        }
        let packet_pid = (((data[off + 1] & 0x1F) as u16) << 8) | data[off + 2] as u16;
        if packet_pid != pid {
            continue;
        }
        let afc = (data[off + 3] & 0x30) >> 4;
        if afc == 0 || afc == 2 {
            continue;
        }
        let pi = if afc == 1 {
            4
        } else {
            5 + data[off + 4] as usize
        };
        if pi >= TS_PACKET {
            continue;
        }
        out.push((off, data[off + 1] & 0x40 != 0, pi));
    }
    out
}

/// 收集指定 PID 上的 PSI section（够解析 PAT/PMT 即可）。
fn sections(data: &[u8], pid: u16) -> Vec<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    let mut out = Vec::new();
    for (off, pusi, pi) in payload_spans(data, pid) {
        if pusi {
            if !buf.is_empty() {
                out.push(std::mem::take(&mut buf));
            }
            let ptr = data[off + pi] as usize;
            let start = off + pi + 1 + ptr;
            if start <= off + TS_PACKET {
                buf.extend_from_slice(&data[start..off + TS_PACKET]);
            }
        } else {
            buf.extend_from_slice(&data[off + pi..off + TS_PACKET]);
        }
        if buf.len() >= 3 {
            let need = 3 + ((((buf[1] & 0x0F) as usize) << 8) | buf[2] as usize);
            if buf.len() >= need {
                out.push(buf[..need].to_vec());
                buf.clear();
            }
        }
    }
    out
}

fn pmt_pids(data: &[u8]) -> Vec<u16> {
    let mut pids = Vec::new();
    for sec in sections(data, 0) {
        if sec.len() < 12 || sec[0] != 0x00 {
            continue;
        }
        let section_len = (((sec[1] & 0x0F) as usize) << 8) | sec[2] as usize;
        let body_end = match (3 + section_len).checked_sub(4) {
            Some(v) if v <= sec.len() => v,
            _ => continue,
        };
        let body = &sec[8..body_end];
        let mut i = 0;
        while i + 3 < body.len() {
            let program = ((body[i] as u16) << 8) | body[i + 1] as u16;
            let pid = (((body[i + 2] & 0x1F) as u16) << 8) | body[i + 3] as u16;
            if program != 0 {
                pids.push(pid);
            }
            i += 4;
        }
    }
    pids
}

/// 解析 PAT/PMT，返回第一个 H.264 (stream_type 0x1B) 的 PID。
pub fn find_video_pid(data: &[u8]) -> Option<u16> {
    for pmt_pid in pmt_pids(data).into_iter().take(4) {
        for sec in sections(data, pmt_pid) {
            if sec.len() < 12 || sec[0] != 0x02 {
                continue;
            }
            let section_len = (((sec[1] & 0x0F) as usize) << 8) | sec[2] as usize;
            let body_end = match (3 + section_len).checked_sub(4) {
                Some(v) if v <= sec.len() => v,
                _ => continue,
            };
            let program_info_len = (((sec[10] & 0x0F) as usize) << 8) | sec[11] as usize;
            let mut i = 12 + program_info_len;
            while i + 5 <= body_end {
                let stream_type = sec[i];
                let es_pid = (((sec[i + 1] & 0x1F) as u16) << 8) | sec[i + 2] as u16;
                let es_info_len = (((sec[i + 3] & 0x0F) as usize) << 8) | sec[i + 4] as usize;
                if stream_type == STREAM_TYPE_H264 {
                    return Some(es_pid);
                }
                i += 5 + es_info_len;
            }
        }
    }
    None
}
