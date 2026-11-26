//! 央视频直播 `encrypt=2`（1080P/720P 同一算法）解密。
//!
//! * 只处理 H.264 slice NAL（type 1/5），其它 NAL 原样保留；
//! * 先去防竞争字节得到 RBSP，在 RBSP 下标上解密，输出保持 RBSP（不再转义）；
//! * TEA-16：key = RBSP[1:17]（type 1）/ RBSP[5:21]（type 5）；
//! * stride = 160 + 32 * (key[0] & 3)，起点 32，每格 8 字节，尾部余 17 字节；
//! * NAL 变短后重建 PES 并按 188 字节重新切包（修正 PES_packet_length）。

use crate::{
    config::{DEFAULT_VIDEO_PID, TS_PACKET},
    media::{tea, ts, Stats},
};

pub fn is_slice(nal: &[u8]) -> bool {
    !nal.is_empty() && matches!(nal[0] & 0x1F, 1 | 5)
}

/// 去掉防竞争字节（00 00 03 -> 00 00）。
pub fn unescape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if i + 2 < data.len() && data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 3 {
            out.push(0);
            out.push(0);
            i += 3;
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    out
}

/// 重新插入防竞争字节（仅测试/构造用；解密输出绝不调用它）。
pub fn escape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut zeros = 0;
    for &byte in data {
        if zeros >= 2 && byte <= 3 {
            out.push(3);
            zeros = 0;
        }
        out.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    out
}

/// 解密一个 slice NAL，返回明文 RBSP（可能比输入短）。
pub fn decrypt_nal(nal: &[u8]) -> Vec<u8> {
    if !is_slice(nal) || nal.len() < 40 {
        return nal.to_vec();
    }
    let nal_type = nal[0] & 0x1F;
    let mut rbsp = unescape(nal);
    let key_off = if nal_type == 1 { 1 } else { 5 };
    if rbsp.len() < key_off + 16 {
        return nal.to_vec();
    }
    let mut key = [0u8; 16];
    key.copy_from_slice(&rbsp[key_off..key_off + 16]);
    let stride = 160 + 32 * (key[0] & 3) as usize;

    let mut offset = 32;
    while offset + 17 <= rbsp.len() {
        let mut block: [u8; 8] = rbsp[offset..offset + 8].try_into().unwrap();
        tea::decrypt_block(&mut block, &key);
        rbsp[offset..offset + 8].copy_from_slice(&block);
        offset += stride;
    }
    rbsp
}

/// 把 ES 切成 (start_code, nal)。
fn split_nals(es: &[u8]) -> Vec<(&[u8], &[u8])> {
    let n = es.len();
    let mut starts: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i + 3 < n {
        if i + 4 <= n && &es[i..i + 4] == b"\x00\x00\x00\x01" {
            starts.push((i, 4));
            i += 4;
        } else if &es[i..i + 3] == b"\x00\x00\x01" {
            starts.push((i, 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, (pos, sc)) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map(|s| s.0).unwrap_or(n);
        out.push((&es[*pos..*pos + *sc], &es[*pos + *sc..end]));
    }
    out
}

fn decrypt_pes(pes: &[u8], stats: &mut Stats) -> Vec<u8> {
    if pes.len() < 9 || &pes[..3] != b"\x00\x00\x01" {
        return pes.to_vec();
    }
    let header_len = 9 + pes[8] as usize;
    if header_len > pes.len() {
        return pes.to_vec();
    }
    let es = &pes[header_len..];
    let mut rebuilt = Vec::with_capacity(es.len());
    for (start_code, nal) in split_nals(es) {
        if is_slice(nal) {
            stats.nals += 1;
            let out = decrypt_nal(nal);
            if out != nal {
                stats.decrypted += 1;
                stats.delta += out.len() as i64 - nal.len() as i64;
            }
            rebuilt.extend_from_slice(start_code);
            rebuilt.extend_from_slice(&out);
        } else {
            rebuilt.extend_from_slice(start_code);
            rebuilt.extend_from_slice(nal);
        }
    }

    let mut header = pes[..header_len].to_vec();
    let mut body_len = rebuilt.len() + header_len - 6;
    if body_len > 0xFFFF {
        body_len = 0; // 视频 PES 允许 0 = 不限定长度
    }
    header[4] = (body_len >> 8) as u8;
    header[5] = body_len as u8;
    let mut out = header;
    out.extend_from_slice(&rebuilt);
    out
}

fn push_packet(out: &mut Vec<u8>, pid: u16, pusi: bool, af: &[u8], payload: &[u8], cc: u8) {
    let afc: u8 = if af.is_empty() { 1 } else { 3 };
    out.push(0x47);
    out.push((if pusi { 0x40 } else { 0 }) | (((pid >> 8) as u8) & 0x1F));
    out.push((pid & 0xFF) as u8);
    out.push((afc << 4) | (cc & 0x0F));
    out.extend_from_slice(af);
    out.extend_from_slice(payload);
}

fn repacketize(pid: u16, first_af: &[u8], pes: &[u8], mut cc: u8, out: &mut Vec<u8>) -> u8 {
    let mut af = first_af.to_vec();
    if af.len() > 184 {
        af.truncate(184);
    }
    let mut capacity = TS_PACKET - 4 - af.len();
    if pes.len() < capacity {
        let extra = capacity - pes.len();
        if af.is_empty() {
            let afl = extra - 1;
            af = vec![afl as u8];
            af.extend(std::iter::repeat(0u8).take(extra - 1));
        } else {
            let afl = af[0] as usize + extra;
            let mut new_af = Vec::with_capacity(af.len() + extra);
            new_af.push(afl as u8);
            new_af.extend_from_slice(&af[1..]);
            new_af.extend(std::iter::repeat(0u8).take(extra));
            af = new_af;
        }
        capacity = pes.len();
    }
    push_packet(out, pid, true, &af, &pes[..capacity], cc);
    cc = (cc + 1) & 0x0F;

    let mut pos = capacity;
    while pos < pes.len() {
        let take = (TS_PACKET - 4).min(pes.len() - pos);
        let af2: Vec<u8> = if take == TS_PACKET - 4 {
            Vec::new()
        } else {
            let afl = TS_PACKET - 5 - take;
            let mut v = Vec::with_capacity(afl + 1);
            v.push(afl as u8);
            v.extend(std::iter::repeat(0u8).take(afl));
            v
        };
        push_packet(out, pid, false, &af2, &pes[pos..pos + take], cc);
        cc = (cc + 1) & 0x0F;
        pos += take;
    }
    cc
}

enum Item<'a> {
    Raw(&'a [u8]),
    Pes {
        cc: u8,
        af: &'a [u8],
        payloads: Vec<&'a [u8]>,
    },
}

/// 解密 TS 中的视频 PES 并重新封装；输出长度可能与输入不同。
pub fn decrypt_ts(data: &[u8], pid: Option<u16>) -> (Vec<u8>, Stats) {
    let pid = pid.unwrap_or_else(|| ts::find_video_pid(data).unwrap_or(DEFAULT_VIDEO_PID));
    let mut stats = Stats::default();
    let mut items: Vec<Item<'_>> = Vec::new();
    let mut current: Option<(u8, &[u8], Vec<&[u8]>)> = None;

    let packets = data.len() / TS_PACKET;
    for i in 0..packets {
        let off = i * TS_PACKET;
        let raw = &data[off..off + TS_PACKET];
        if raw[0] != 0x47 {
            items.push(Item::Raw(raw));
            continue;
        }
        let packet_pid = (((raw[1] & 0x1F) as u16) << 8) | raw[2] as u16;
        let pusi = raw[1] & 0x40 != 0;
        let afc = (raw[3] & 0x30) >> 4;
        let cc = raw[3] & 0x0F;
        let (af, payload): (&[u8], &[u8]) = match afc {
            1 => (&[], &raw[4..]),
            2 | 3 => {
                let af_end = (5 + raw[4] as usize).min(TS_PACKET);
                let af = &raw[4..af_end];
                let payload = if afc == 3 && af_end < TS_PACKET {
                    &raw[af_end..]
                } else {
                    &[]
                };
                (af, payload)
            }
            _ => (&[], &[]),
        };
        if packet_pid != pid || afc == 0 || afc == 2 {
            items.push(Item::Raw(raw));
            continue;
        }
        if pusi {
            if let Some((prev_cc, prev_af, pls)) = current.take() {
                items.push(Item::Pes { cc: prev_cc, af: prev_af, payloads: pls });
            }
            current = Some((cc, af, vec![payload]));
        } else if let Some((_, _, pls)) = current.as_mut() {
            pls.push(payload);
        } else {
            items.push(Item::Raw(raw));
        }
    }
    if let Some((prev_cc, prev_af, pls)) = current.take() {
        items.push(Item::Pes { cc: prev_cc, af: prev_af, payloads: pls });
    }

    let mut out = Vec::with_capacity(data.len());
    let mut next_cc: Option<u8> = None;
    for item in items {
        match item {
            Item::Raw(raw) => out.extend_from_slice(raw),
            Item::Pes { cc, af, payloads } => {
                stats.pes += 1;
                let mut pes = Vec::new();
                for payload in payloads {
                    pes.extend_from_slice(payload);
                }
                let new_pes = decrypt_pes(&pes, &mut stats);
                let start_cc = *next_cc.get_or_insert(cc);
                next_cc = Some(repacketize(pid, af, &new_pes, start_cc, &mut out));
            }
        }
    }
    let tail = data.len() % TS_PACKET;
    if tail > 0 {
        out.extend_from_slice(&data[data.len() - tail..]);
    }
    (out, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: usize = 32;
    const GUARD: usize = 17;
    const CELL: usize = 8;

    fn make_slice(nal_type: u8, stride_sel: u8) -> (Vec<u8>, Vec<u8>, usize) {
        let key_off = if nal_type == 1 { 1 } else { 5 };
        let filler: Vec<u8> = (0x11..=0xA0u8).collect();
        let mut key = [0u8; 16];
        key[0] = stride_sel & 3;
        for i in 1..16 {
            key[i] = 0x11 + i as u8;
        }
        let stride = 160 + 32 * (key[0] & 3) as usize;

        let total = START + stride * 3 + GUARD + CELL;
        let mut body = vec![0u8; total];
        for (i, slot) in body.iter_mut().enumerate() {
            *slot = filler[i % filler.len()];
        }
        body[0] = nal_type | if nal_type == 5 { 0x60 } else { 0x40 };
        body[key_off..key_off + 16].copy_from_slice(&key);
        let plain = body.clone();

        let mut enc = body;
        let mut o = START;
        while o + CELL <= enc.len() {
            let mut block: [u8; 8] = enc[o..o + 8].try_into().unwrap();
            tea::encrypt_block(&mut block, &key);
            enc[o..o + 8].copy_from_slice(&block);
            o += stride;
        }
        (plain, escape(&enc), stride)
    }

    #[test]
    fn type1_and_type5_roundtrip() {
        for sel in 0..4u8 {
            for nal_type in [1u8, 5] {
                let (plain, ebsp, stride) = make_slice(nal_type, sel);
                assert_eq!(stride, 160 + 32 * sel as usize);
                assert_eq!(decrypt_nal(&ebsp), plain);
            }
        }
    }

    #[test]
    fn epb_is_removed_not_reinserted() {
        let (plain, _, _) = make_slice(1, 0);
        let mut patched = plain.clone();
        patched[20..23].copy_from_slice(b"\x00\x00\x02");
        let key: [u8; 16] = patched[1..17].try_into().unwrap();
        let stride = 160 + 32 * (key[0] & 3) as usize;
        let mut enc = patched.clone();
        let mut o = START;
        while o + GUARD <= enc.len() {
            let mut block: [u8; 8] = enc[o..o + 8].try_into().unwrap();
            tea::encrypt_block(&mut block, &key);
            enc[o..o + 8].copy_from_slice(&block);
            o += stride;
        }
        let ebsp = escape(&enc);
        assert_eq!(ebsp.len(), patched.len() + 1);
        let out = decrypt_nal(&ebsp);
        assert_eq!(out, patched);
    }

    #[test]
    fn non_slice_and_short_untouched() {
        let nal = [0x67u8].iter().copied().chain(std::iter::repeat(0x42).take(100)).collect::<Vec<_>>();
        assert_eq!(decrypt_nal(&nal), nal);
        let short = [0x41u8].iter().copied().chain(std::iter::repeat(0).take(20)).collect::<Vec<_>>();
        assert_eq!(decrypt_nal(&short), short);
    }
}
