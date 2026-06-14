//! Binary arithmetic coder (carry-less, 32-bit), lpaq-style.
//!
//! Part of the EDITABLE algorithm. Probabilities are 12-bit P(bit==1) in
//! [1, 4094]. Encoder appends to a Vec; decoder reads from a slice.

pub struct Encoder {
    x1: u32,
    x2: u32,
    pub out: Vec<u8>,
}

impl Encoder {
    pub fn new() -> Self {
        Encoder { x1: 0, x2: 0xffff_ffff, out: Vec::new() }
    }

    #[inline]
    pub fn encode(&mut self, mut p: i32, bit: i32) {
        if p < 1 { p = 1; }
        if p > 4094 { p = 4094; }
        let range = (self.x2 - self.x1) as u64;
        let xmid = self.x1.wrapping_add(((range * p as u64) >> 12) as u32);
        if bit != 0 { self.x2 = xmid; } else { self.x1 = xmid + 1; }
        while (self.x1 ^ self.x2) & 0xff00_0000 == 0 {
            self.out.push((self.x2 >> 24) as u8);
            self.x1 <<= 8;
            self.x2 = (self.x2 << 8) | 255;
        }
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.out.push((self.x1 >> 24) as u8);
        self.out.push((self.x1 >> 16) as u8);
        self.out.push((self.x1 >> 8) as u8);
        self.out.push(self.x1 as u8);
        self.out
    }
}

pub struct Decoder<'a> {
    x1: u32,
    x2: u32,
    x: u32,
    inp: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(inp: &'a [u8]) -> Self {
        let mut d = Decoder { x1: 0, x2: 0xffff_ffff, x: 0, inp, pos: 0 };
        for _ in 0..4 {
            d.x = (d.x << 8) | d.getc();
        }
        d
    }

    #[inline]
    fn getc(&mut self) -> u32 {
        if self.pos < self.inp.len() {
            let b = self.inp[self.pos] as u32;
            self.pos += 1;
            b
        } else {
            255 // past-end padding (matches reference behavior)
        }
    }

    #[inline]
    pub fn decode(&mut self, mut p: i32) -> i32 {
        if p < 1 { p = 1; }
        if p > 4094 { p = 4094; }
        let range = (self.x2 - self.x1) as u64;
        let xmid = self.x1.wrapping_add(((range * p as u64) >> 12) as u32);
        let bit = if self.x <= xmid { 1 } else { 0 };
        if bit != 0 { self.x2 = xmid; } else { self.x1 = xmid + 1; }
        while (self.x1 ^ self.x2) & 0xff00_0000 == 0 {
            self.x1 <<= 8;
            self.x2 = (self.x2 << 8) | 255;
            self.x = (self.x << 8) | self.getc();
        }
        bit
    }
}
