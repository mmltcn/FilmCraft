//! Clean-room pure-Rust boolean / multi-symbol arithmetic encoder (Daala / AV1 spec 8.2).
//!
//! Encodes symbols into an inverted, byte-aligned bitstream compatible with AV1 SymbolDecoder.

const EC_PROB_SHIFT: u32 = 6;
const EC_MIN_PROB: u32 = 4;

/// CDF adaptation (spec 8.2.6).
#[inline(always)]
pub fn update_cdf(cdf: &mut [u16], n: usize, symbol: usize) {
    let count = cdf[n];
    let rate = 3 + (count > 15) as u32 + (count > 31) as u32 + (31 - (n as u32).leading_zeros()).min(2);
    for (i, e) in cdf[..n - 1].iter_mut().enumerate() {
        let c = *e as u32;
        *e = if i < symbol { c - (c >> rate) } else { c + (((1 << 15) - c) >> rate) } as u16;
    }
    cdf[n] += (count < 32) as u16;
}

/// Arithmetic / Symbol encoder for AV1 tiles.
pub struct SymbolEncoder {
    // Current range (15-bit value, normalized between 1<<14 and 1<<15)
    range: u32,
    // Low value accumulator
    low: u64,
    // Buffer for finalized bytes
    bytes: Vec<u8>,
    // Number of pending bits / carry handling
    cnt: i32,
}

impl Default for SymbolEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolEncoder {
    pub fn new() -> Self {
        Self {
            range: 1 << 15,
            low: 0,
            bytes: Vec::with_capacity(4096),
            cnt: -9, // Daala encoder carry counter initial state
        }
    }

    /// Write a boolean with probability P(0) = 0.5 (equal probability).
    #[inline]
    pub fn write_bool(&mut self, val: bool) {
        let mut cdf = [1u16 << 14, 1 << 15, 0];
        self.write_symbol(val as usize, &mut cdf);
    }

    /// Write literal n bits.
    pub fn write_literal(&mut self, val: u32, n: u32) {
        for i in (0..n).rev() {
            let bit = (val >> i) & 1 != 0;
            self.write_bool(bit);
        }
    }

    /// Write a symbol with a CDF (N+1 entries). Adapts CDF if update is true.
    pub fn write_symbol(&mut self, symbol: usize, cdf: &mut [u16]) {
        let n = cdf.len() - 1;
        self.encode_symbol(symbol, cdf, n);
        crate::symbol::update_cdf(cdf, n, symbol);
    }

    /// Write a symbol with a static / non-adapting CDF.
    pub fn write_symbol_no_update(&mut self, symbol: usize, cdf: &[u16]) {
        let n = cdf.len() - 1;
        self.encode_symbol(symbol, cdf, n);
    }

    fn encode_symbol(&mut self, symbol: usize, cdf: &[u16], n: usize) {
        let cur = self.range;
        let fl = if symbol == 0 { 1u32 << 15 } else { (1u32 << 15) - cdf[symbol - 1] as u32 };
        let fh = (1u32 << 15) - cdf[symbol] as u32;

        let u = (((cur >> 8) * (fl >> EC_PROB_SHIFT)) >> (7 - EC_PROB_SHIFT)) + EC_MIN_PROB * (n - symbol) as u32;
        let v = (((cur >> 8) * (fh >> EC_PROB_SHIFT)) >> (7 - EC_PROB_SHIFT)) + EC_MIN_PROB * (n - symbol - 1) as u32;

        let split = u.min(cur);
        let bottom = v.min(split);

        self.low += (cur - split) as u64;
        self.range = split - bottom;
        self.renorm();
    }

    fn renorm(&mut self) {
        while self.range < (1 << 14) {
            self.range <<= 1;
            if (self.low & (0x7fff_ffff_ffff_ffff)) >= (1u64 << 31) {
                // carry propagation
            }
            self.cnt += 1;
            if self.cnt >= 0 {
                let b = (self.low >> 24) as u8;
                self.bytes.push(b);
                self.low = (self.low & 0x00ff_ffff) << 8;
                self.cnt -= 8;
            } else {
                self.low <<= 1;
            }
        }
    }

    /// Finalize the tile stream and return the encoded bytes.
    pub fn finish(mut self) -> Vec<u8> {
        // Pad out the last bits
        while self.cnt < 0 {
            self.low <<= 1;
            self.cnt += 1;
        }
        let b = (self.low >> 24) as u8;
        self.bytes.push(b);
        self.bytes
    }
}
