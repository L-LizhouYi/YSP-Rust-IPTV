//! 720P/540P 直播线路（h5e classic 网格 + TEA-16）。
//!
//! * 视频 slice（type 1/5）：key = RBSP[16:32]，起点 32，stride 80，每格 8 字节；
//! * 尾部条件：剩余一个完整 8 字节格（guard = 8）；
//! * 短于 129 字节的 NAL 不加密、不解密；
//! * EPB(00 00 03) 按 RBSP 索引读写，解密后删除 03；
//! * NAL 变短的差额用 TS adaptation field 填充补回，文件总长度不变。

use crate::{
    config::{DEFAULT_VIDEO_PID, TS_PACKET},
    media::{tea, ts, Stats},
};

const START: usize = 32;
const STRIDE: usize = 80;
const KEY_OFF: usize = 16;
const CELL: usize = 8;
const TAIL_GUARD: usize = 8;
const MIN_NAL_LEN: usize = 129;

/// RBSP -> EBSP 位置映射（EPB 的 0x03 不占 RBSP 位置）。
fn build_rbsp_map(nal: &[u8]) -> Vec<u32> {
    let mut map = Vec::with_capacity(nal.len());
    let mut i = 0;
    while i < nal.len() {
        if i + 2 < nal.len() && nal[i] == 0 && nal[i + 1] == 0 && nal[i + 2] == 3 {
            map.push(i as u32);
            map.push((i + 1) as u32);
            i += 3;
        } else {
            map.push(i as u32);
            i += 1;
        }
    }
    map
}

fn collect_epb_positions(nal: &[u8]) -> Vec<u32> {
    let mut out = Vec::new();
    if nal.len() < 3 {
        return out;
    }
    for i in 0..nal.len() - 2 {
        if nal[i] == 0 && nal[i + 1] == 0 && nal[i + 2] == 3 {
            out.push(i as u32);
        }
    }
    out
}

/// 原地解密一个 NAL，返回 (新数据, 处理格数, 删除的 EPB 数)。
fn grid(nal: &[u8]) -> (Vec<u8>, usize, usize) {
    if nal.len() < MIN_NAL_LEN {
        return (nal.to_vec(), 0, 0);
    }
    let epbs = collect_epb_positions(nal);
    let r2e = build_rbsp_map(nal);
    let rbsp_len = r2e.len();
    if rbsp_len < START {
        return (nal.to_vec(), 0, 0);
    }

    let mut work = nal.to_vec();
    let mut key = [0u8; 16];
    for (b, slot) in key.iter_mut().enumerate() {
        *slot = nal[r2e[KEY_OFF + b] as usize];
    }

    let mut cells = 0usize;
    loop {
        let o = START + cells * STRIDE;
        if o + TAIL_GUARD > rbsp_len || o + CELL > rbsp_len {
            break;
        }
        let mut block = [0u8; CELL];
        for b in 0..CELL {
            block[b] = work[r2e[o + b] as usize];
        }
        tea::decrypt_block(&mut block, &key);
        for b in 0..CELL {
            work[r2e[o + b] as usize] = block[b];
        }
        cells += 1;
    }

    if epbs.is_empty() || cells == 0 {
        return (work, cells, 0);
    }

    // 从后往前删 EPB 的 0x03（等价 Python drop_epb_03）
    let mut drop = vec![false; work.len()];
    for e in epbs.iter().rev() {
        let idx = *e as usize;
        if idx + 2 >= work.len() {
            continue;
        }
        drop[idx + 2] = true;
    }
    let mut out = Vec::with_capacity(work.len() - epbs.len());
    for (i, byte) in work.into_iter().enumerate() {
        if !drop[i] {
            out.push(byte);
        }
    }
    (out, cells, epbs.len())
}

#[derive(Default)]
struct Session {
    stats: Stats,
}

impl Session {
    fn on_nal(&mut self, nal: &[u8]) -> Vec<u8> {
        if nal.len() < MIN_NAL_LEN || !matches!(nal[0] & 0x1F, 1 | 5) {
            return nal.to_vec();
        }
        let (out, cells, dropped) = grid(nal);
        self.stats.nals += 1;
        self.stats.cells += cells;
        self.stats.epb_dropped += dropped;
        out
    }
}

/// 扩展单个 TS 包的 adaptation field，返回从 payload 腾出的字节数。
fn expand_af_steal(data: &mut [u8], pkt_off: usize, need: usize) -> usize {
    if need == 0 || pkt_off + TS_PACKET > data.len() {
        return 0;
    }
    let afc = (data[pkt_off + 3] & 0x30) >> 4;
    if afc == 1 {
        let af_len = (need - 1).min(182);
        let steal = 1 + af_len;
        let old_payload = data[pkt_off + 4..pkt_off + TS_PACKET].to_vec();
        data[pkt_off + 3] = (data[pkt_off + 3] & 0xCF) | 0x30; // afc = 3
        data[pkt_off + 4] = af_len as u8;
        if af_len > 0 {
            data[pkt_off + 5] = 0x00;
            for i in 1..af_len {
                data[pkt_off + 5 + i] = 0xFF;
            }
        }
        let new_pl = 184 - steal;
        let dst = pkt_off + 5 + af_len;
        data[dst..dst + new_pl].copy_from_slice(&old_payload[..new_pl]);
        return steal;
    }

    if afc == 2 || afc == 3 {
        let af_len = data[pkt_off + 4] as usize;
        let pi = 5 + af_len;
        if pi >= TS_PACKET {
            return 0;
        }
        let old_payload_len = TS_PACKET - pi;
        let mut add = need.min(old_payload_len);
        if add == 0 {
            return 0;
        }
        if af_len + add > 182 {
            add = 182 - af_len;
            if add == 0 {
                return 0;
            }
        }
        let new_af_len = af_len + add;
        let old_payload = data[pkt_off + pi..pkt_off + TS_PACKET].to_vec();
        for i in 0..add {
            data[pkt_off + 5 + af_len + i] = 0xFF;
        }
        data[pkt_off + 4] = new_af_len as u8;
        let new_pl = old_payload_len - add;
        let dst = pkt_off + 5 + new_af_len;
        data[dst..dst + new_pl].copy_from_slice(&old_payload[..new_pl]);
        data[pkt_off + 3] =
            (data[pkt_off + 3] & 0xCF) | if new_pl == 0 { 0x20 } else { 0x30 };
        return add;
    }
    0
}

fn flush(
    data: &mut [u8],
    pes: &mut Vec<u8>,
    spans: &mut Vec<(usize, usize, usize)>,
    session: &mut Session,
) {
    if pes.is_empty() {
        return;
    }

    let base_skip = if pes.len() >= 9 && &pes[..3] == b"\x00\x00\x01" {
        9 + pes[8] as usize
    } else {
        0
    };
    if base_skip > pes.len() {
        pes.clear();
        spans.clear();
        return;
    }

    let pes_hdr = pes[..base_skip].to_vec();
    let es = &pes[base_skip..];
    let es_len = es.len();

    let mut starts: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i + 3 < es_len {
        if i + 4 <= es_len && &es[i..i + 4] == b"\x00\x00\x00\x01" {
            starts.push((i, 4));
            i += 4;
        } else if &es[i..i + 3] == b"\x00\x00\x01" {
            starts.push((i, 3));
            i += 3;
        } else {
            i += 1;
        }
    }

    let mut new_es: Vec<u8> = Vec::with_capacity(es_len);
    let mut cursor = 0usize;
    for (idx, start) in starts.iter().enumerate() {
        let (pos, sc) = *start;
        let end = starts.get(idx + 1).map(|s| s.0).unwrap_or(es_len);
        if cursor < pos {
            new_es.extend_from_slice(&es[cursor..pos]);
        }
        new_es.extend_from_slice(&es[pos..pos + sc]);
        if pos + sc >= end {
            cursor = end;
            continue;
        }
        let nal = &es[pos + sc..end];
        let out = session.on_nal(nal);
        new_es.extend_from_slice(&out);
        cursor = end;
    }
    if cursor < es_len {
        new_es.extend_from_slice(&es[cursor..]);
    }

    let mut new_pes = pes_hdr;
    new_pes.extend_from_slice(&new_es);

    // PES_packet_length 必须跟着 ES 收缩一起改，否则 ffmpeg 报
    // "PES packet size mismatch"。
    if base_skip >= 6 {
        let old_len = ((new_pes[4] as usize) << 8) | new_pes[5] as usize;
        if old_len != 0 {
            let mut new_len = new_pes.len() - 6;
            if new_len > 0xFFFF {
                new_len = 0;
            }
            new_pes[4] = (new_len >> 8) as u8;
            new_pes[5] = new_len as u8;
        }
    }

    let capacity: usize = spans.iter().map(|s| s.2).sum();
    if capacity > new_pes.len() {
        let mut remaining = capacity - new_pes.len();
        for span in spans.iter().rev() {
            if remaining == 0 {
                break;
            }
            let stolen = expand_af_steal(data, span.0, remaining);
            remaining = remaining.saturating_sub(stolen);
        }

        let mut rebuilt = Vec::with_capacity(spans.len());
        for (pkt_off, _, _) in spans.iter() {
            let off = *pkt_off;
            let afc = (data[off + 3] & 0x30) >> 4;
            if afc == 0 || afc == 2 {
                continue;
            }
            let pi = if afc == 1 { 4 } else { 5 + data[off + 4] as usize };
            if pi >= TS_PACKET {
                continue;
            }
            rebuilt.push((off, pi, TS_PACKET - pi));
        }
        *spans = rebuilt;
    }

    let mut cursor = 0usize;
    for (pkt_off, pi, pl) in spans.iter() {
        let dst = *pkt_off + *pi;
        let chunk = if cursor < new_pes.len() {
            new_pes.len() - cursor
        } else {
            0
        };
        let chunk = chunk.min(*pl);
        if chunk > 0 {
            data[dst..dst + chunk].copy_from_slice(&new_pes[cursor..cursor + chunk]);
        }
        for byte in data[dst + chunk..dst + *pl].iter_mut() {
            *byte = 0xFF;
        }
        cursor += *pl;
    }

    pes.clear();
    spans.clear();
}

fn decrypt_ts_inplace(data: &mut [u8], session: &mut Session, vpid: u16) {
    if data.len() < TS_PACKET {
        return;
    }
    let mut pes: Vec<u8> = Vec::new();
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    let packets = data.len() / TS_PACKET;
    for i in 0..packets {
        let off = i * TS_PACKET;
        if data[off] != 0x47 {
            continue;
        }
        let packet_pid = (((data[off + 1] & 0x1F) as u16) << 8) | data[off + 2] as u16;
        if packet_pid != vpid {
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
        if data[off + 1] & 0x40 != 0 {
            flush(data, &mut pes, &mut spans, session);
        }
        pes.extend_from_slice(&data[off + pi..off + TS_PACKET]);
        spans.push((off, pi, TS_PACKET - pi));
    }
    flush(data, &mut pes, &mut spans, session);
}

/// 复制式解密，返回 (明文 TS, 统计)。`vpid=None` 时自动探测。
pub fn decrypt_ts(data: &[u8], vpid: Option<u16>) -> (Vec<u8>, Stats) {
    let mut out = data.to_vec();
    if out.len() < TS_PACKET {
        return (out, Stats::default());
    }
    let pid = vpid
        .or_else(|| ts::find_video_pid(&out))
        .unwrap_or(DEFAULT_VIDEO_PID);
    let mut session = Session::default();
    decrypt_ts_inplace(&mut out, &mut session, pid);
    (out, session.stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_grid_roundtrip() {
        let mut key = [0u8; 16];
        for (i, slot) in key.iter_mut().enumerate() {
            *slot = 0x30 + i as u8;
        }
        let mut nal = vec![0x41u8];
        nal.extend((0u8..=255).cycle().take(400));
        nal[KEY_OFF..KEY_OFF + 16].copy_from_slice(&key);
        let plain = nal.clone();
        for o in (START..nal.len() - CELL + 1).step_by(STRIDE) {
            let mut block: [u8; 8] = nal[o..o + 8].try_into().unwrap();
            tea::encrypt_block(&mut block, &key);
            nal[o..o + 8].copy_from_slice(&block);
        }
        let (out, cells, _) = grid(&nal);
        assert!(cells > 0);
        assert_eq!(out, plain);
    }

    #[test]
    fn short_nal_untouched() {
        let nal = vec![0x41u8; 100];
        let (out, cells, _) = grid(&nal);
        assert_eq!(cells, 0);
        assert_eq!(out, nal);
    }
}
