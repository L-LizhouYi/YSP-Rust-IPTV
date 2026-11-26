//! TEA-16（小端 32 位字，delta 0x9E3779B9）。两个直播解密算法共用。

pub const DELTA: u32 = 0x9E37_79B9;

#[inline]
pub fn decrypt_block(block: &mut [u8; 8], key: &[u8; 16]) {
    let mut v0 = u32::from_le_bytes([block[0], block[1], block[2], block[3]]);
    let mut v1 = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let k0 = u32::from_le_bytes([key[0], key[1], key[2], key[3]]);
    let k1 = u32::from_le_bytes([key[4], key[5], key[6], key[7]]);
    let k2 = u32::from_le_bytes([key[8], key[9], key[10], key[11]]);
    let k3 = u32::from_le_bytes([key[12], key[13], key[14], key[15]]);
    let mut total = DELTA.wrapping_mul(16);
    for _ in 0..16 {
        let t = ((v0 << 4).wrapping_add(k2))
            ^ (v0.wrapping_add(total))
            ^ ((v0 >> 5).wrapping_add(k3));
        v1 = v1.wrapping_sub(t);
        let t = ((v1 << 4).wrapping_add(k0))
            ^ (v1.wrapping_add(total))
            ^ ((v1 >> 5).wrapping_add(k1));
        v0 = v0.wrapping_sub(t);
        total = total.wrapping_sub(DELTA);
    }
    block[0..4].copy_from_slice(&v0.to_le_bytes());
    block[4..8].copy_from_slice(&v1.to_le_bytes());
}

/// 仅供单元测试使用的加密逆运算。
#[cfg(test)]
pub fn encrypt_block(block: &mut [u8; 8], key: &[u8; 16]) {
    let mut v0 = u32::from_le_bytes([block[0], block[1], block[2], block[3]]);
    let mut v1 = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let k0 = u32::from_le_bytes([key[0], key[1], key[2], key[3]]);
    let k1 = u32::from_le_bytes([key[4], key[5], key[6], key[7]]);
    let k2 = u32::from_le_bytes([key[8], key[9], key[10], key[11]]);
    let k3 = u32::from_le_bytes([key[12], key[13], key[14], key[15]]);
    let mut total = 0u32;
    for _ in 0..16 {
        total = total.wrapping_add(DELTA);
        let t = ((v1 << 4).wrapping_add(k0))
            ^ (v1.wrapping_add(total))
            ^ ((v1 >> 5).wrapping_add(k1));
        v0 = v0.wrapping_add(t);
        let t = ((v0 << 4).wrapping_add(k2))
            ^ (v0.wrapping_add(total))
            ^ ((v0 >> 5).wrapping_add(k3));
        v1 = v1.wrapping_add(t);
    }
    block[0..4].copy_from_slice(&v0.to_le_bytes());
    block[4..8].copy_from_slice(&v1.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key: [u8; 16] = std::array::from_fn(|i| i as u8);
        let plain = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let mut block = plain;
        encrypt_block(&mut block, &key);
        assert_ne!(block, plain);
        decrypt_block(&mut block, &key);
        assert_eq!(block, plain);
    }
}
