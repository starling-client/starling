// Altered from trees.c of zlib 1.3.2 (output deflated data using Huffman coding):
// Copyright (C) 1995-2026 Jean-loup Gailly. detect_data_type() function provided
// freely by Cosmin Truta, 2006. For conditions of distribution and use, see the
// copyright notice in this crate's LICENSE (zlib.h's notice).
//
// This is a Rust port of the parts the default level reaches. It is not the
// original software.

//! Huffman trees and block output: `trees.c`'s `_tr_*` functions.
//!
//! Each tree node keeps zlib's two unions as plain `u16` fields: `fc` is the
//! frequency while the tree is built and the code afterwards, `dl` the parent
//! index while the tree is built and the bit length afterwards. The order in
//! which they are overwritten is zlib's, so ties break the same way.

use crate::bits::BitWriter;

pub(crate) const LENGTH_CODES: usize = 29;
pub(crate) const LITERALS: usize = 256;
pub(crate) const L_CODES: usize = LITERALS + 1 + LENGTH_CODES;
pub(crate) const D_CODES: usize = 30;
const BL_CODES: usize = 19;
const HEAP_SIZE: usize = 2 * L_CODES + 1;
const MAX_BITS: usize = 15;
const MAX_BL_BITS: usize = 7;
const END_BLOCK: usize = 256;
const REP_3_6: usize = 16;
const REPZ_3_10: usize = 17;
const REPZ_11_138: usize = 18;

/// zlib's `lit_bufsize` for memLevel 8: symbols a block holds before it is flushed.
pub(crate) const LIT_BUFSIZE: usize = 1 << (8 + 6);
/// zlib's `sym_end`: the block is flushed when this many symbols are tallied.
const SYM_END: usize = LIT_BUFSIZE - 1;

const STORED_BLOCK: u32 = 0;
const STATIC_TREES: u32 = 1;
const DYN_TREES: u32 = 2;

const EXTRA_LBITS: [u8; LENGTH_CODES] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const EXTRA_DBITS: [u8; D_CODES] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const EXTRA_BLBITS: [u8; BL_CODES] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 3, 7];
const BL_ORDER: [u8; BL_CODES] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// One tree node: `fc` is Freq, then Code; `dl` is Dad, then Len.
#[derive(Clone, Copy, Default)]
pub(crate) struct Ct {
    fc: u16,
    dl: u16,
}

/// The static tables `tr_static_init` builds, computed at compile time.
struct Static {
    ltree: [Ct; L_CODES + 2],
    dtree: [Ct; D_CODES],
    dist_code: [u8; 512],
    length_code: [u8; 256],
    base_length: [u8; LENGTH_CODES],
    base_dist: [u16; D_CODES],
}

const fn bi_reverse(mut code: u32, mut len: u32) -> u16 {
    let mut res = 0u32;
    loop {
        res |= code & 1;
        code >>= 1;
        res <<= 1;
        len -= 1;
        if len == 0 {
            break;
        }
    }
    (res >> 1) as u16
}

const STATIC: Static = {
    let mut s = Static {
        ltree: [Ct { fc: 0, dl: 0 }; L_CODES + 2],
        dtree: [Ct { fc: 0, dl: 0 }; D_CODES],
        dist_code: [0; 512],
        length_code: [0; 256],
        base_length: [0; LENGTH_CODES],
        base_dist: [0; D_CODES],
    };
    let mut length = 0usize;
    let mut code = 0usize;
    while code < LENGTH_CODES - 1 {
        s.base_length[code] = length as u8;
        let mut n = 0;
        while n < (1 << EXTRA_LBITS[code]) {
            s.length_code[length] = code as u8;
            length += 1;
            n += 1;
        }
        code += 1;
    }
    // Length 258 has two encodings; zlib keeps code 285.
    s.length_code[length - 1] = code as u8;
    // zlib leaves this 0 and sends no extra bits for code 285; with 255 here the
    // extra value of length 258 is 0 as well, so `compress_block` can always add it.
    s.base_length[code] = 255;

    let mut dist = 0usize;
    code = 0;
    while code < 16 {
        s.base_dist[code] = dist as u16;
        let mut n = 0;
        while n < (1 << EXTRA_DBITS[code]) {
            s.dist_code[dist] = code as u8;
            dist += 1;
            n += 1;
        }
        code += 1;
    }
    dist >>= 7;
    while code < D_CODES {
        s.base_dist[code] = (dist << 7) as u16;
        let mut n = 0;
        while n < (1 << (EXTRA_DBITS[code] - 7)) {
            s.dist_code[256 + dist] = code as u8;
            dist += 1;
            n += 1;
        }
        code += 1;
    }

    let mut bl_count = [0u16; MAX_BITS + 1];
    let mut n = 0;
    while n <= 287 {
        let len = if n <= 143 {
            8
        } else if n <= 255 {
            9
        } else if n <= 279 {
            7
        } else {
            8
        };
        s.ltree[n].dl = len;
        bl_count[len as usize] += 1;
        n += 1;
    }
    // gen_codes over all 288 codes (286 and 287 make the tree canonical).
    let mut next_code = [0u16; MAX_BITS + 1];
    let mut c = 0u32;
    let mut bits = 1;
    while bits <= MAX_BITS {
        c = (c + bl_count[bits - 1] as u32) << 1;
        next_code[bits] = c as u16;
        bits += 1;
    }
    n = 0;
    while n <= L_CODES + 1 {
        let len = s.ltree[n].dl as usize;
        s.ltree[n].fc = bi_reverse(next_code[len] as u32, len as u32);
        next_code[len] += 1;
        n += 1;
    }
    n = 0;
    while n < D_CODES {
        s.dtree[n] = Ct {
            fc: bi_reverse(n as u32, 5),
            dl: 5,
        };
        n += 1;
    }
    s
};

#[inline(always)]
fn d_code(dist: usize) -> usize {
    if dist < 256 {
        STATIC.dist_code[dist] as usize
    } else {
        STATIC.dist_code[256 + (dist >> 7)] as usize
    }
}

/// `static_tree_desc`: what `build_tree` needs to know about one tree kind.
struct Desc {
    static_tree: Option<&'static [Ct]>,
    extra_bits: &'static [u8],
    extra_base: usize,
    elems: usize,
    max_length: usize,
}

const L_DESC: Desc = Desc {
    static_tree: Some(&STATIC.ltree),
    extra_bits: &EXTRA_LBITS,
    extra_base: LITERALS + 1,
    elems: L_CODES,
    max_length: MAX_BITS,
};
const D_DESC: Desc = Desc {
    static_tree: Some(&STATIC.dtree),
    extra_bits: &EXTRA_DBITS,
    extra_base: 0,
    elems: D_CODES,
    max_length: MAX_BITS,
};
const BL_DESC: Desc = Desc {
    static_tree: None,
    extra_bits: &EXTRA_BLBITS,
    extra_base: 0,
    elems: BL_CODES,
    max_length: MAX_BL_BITS,
};

/// The heap and counters `build_tree` works in (zlib keeps them in `deflate_state`).
struct Heap {
    heap: [u16; HEAP_SIZE],
    heap_len: usize,
    heap_max: usize,
    depth: [u8; HEAP_SIZE],
    bl_count: [u16; MAX_BITS + 1],
    /// Bit length of the block with the dynamic trees. zlib's `ulg`: it wraps
    /// below zero while forced codes are added, and only its final value is used.
    opt_len: u64,
    /// Bit length of the block with the static trees, wrapping like `opt_len`.
    static_len: u64,
}

impl Heap {
    /// `smaller`: frequency, then depth, as the tie breaker.
    #[inline(always)]
    fn smaller(&self, tree: &[Ct], n: usize, m: usize) -> bool {
        tree[n].fc < tree[m].fc || (tree[n].fc == tree[m].fc && self.depth[n] <= self.depth[m])
    }

    fn pqdownheap(&mut self, tree: &[Ct], mut k: usize) {
        let v = self.heap[k] as usize;
        let mut j = k << 1;
        while j <= self.heap_len {
            if j < self.heap_len
                && self.smaller(tree, self.heap[j + 1] as usize, self.heap[j] as usize)
            {
                j += 1;
            }
            if self.smaller(tree, v, self.heap[j] as usize) {
                break;
            }
            self.heap[k] = self.heap[j];
            k = j;
            j <<= 1;
        }
        self.heap[k] = v as u16;
    }

    fn gen_bitlen(&mut self, tree: &mut [Ct], max_code: usize, desc: &Desc) {
        self.bl_count = [0; MAX_BITS + 1];
        let mut overflow = 0i32;
        tree[self.heap[self.heap_max] as usize].dl = 0;

        let mut h = self.heap_max + 1;
        while h < HEAP_SIZE {
            let n = self.heap[h] as usize;
            let mut bits = tree[tree[n].dl as usize].dl as usize + 1;
            if bits > desc.max_length {
                bits = desc.max_length;
                overflow += 1;
            }
            tree[n].dl = bits as u16;
            h += 1;
            if n > max_code {
                continue;
            }
            self.bl_count[bits] += 1;
            let xbits = if n >= desc.extra_base {
                desc.extra_bits[n - desc.extra_base] as u64
            } else {
                0
            };
            let f = tree[n].fc as u64;
            self.opt_len = self.opt_len.wrapping_add(f * (bits as u64 + xbits));
            if let Some(stree) = desc.static_tree {
                self.static_len = self
                    .static_len
                    .wrapping_add(f * (stree[n].dl as u64 + xbits));
            }
        }
        if overflow == 0 {
            return;
        }

        loop {
            let mut bits = desc.max_length - 1;
            while self.bl_count[bits] == 0 {
                bits -= 1;
            }
            self.bl_count[bits] -= 1;
            self.bl_count[bits + 1] += 2;
            self.bl_count[desc.max_length] -= 1;
            overflow -= 2;
            if overflow <= 0 {
                break;
            }
        }

        let mut h = HEAP_SIZE;
        for bits in (1..=desc.max_length).rev() {
            let mut n = self.bl_count[bits];
            while n != 0 {
                h -= 1;
                let m = self.heap[h] as usize;
                if m > max_code {
                    continue;
                }
                if tree[m].dl as usize != bits {
                    self.opt_len = self.opt_len.wrapping_add(
                        (bits as u64)
                            .wrapping_sub(tree[m].dl as u64)
                            .wrapping_mul(tree[m].fc as u64),
                    );
                    tree[m].dl = bits as u16;
                }
                n -= 1;
            }
        }
    }

    /// Builds one Huffman tree, sets each code's length and code, and
    /// returns `max_code`, the largest code with a nonzero frequency.
    fn build_tree(&mut self, tree: &mut [Ct], desc: &Desc) -> usize {
        let elems = desc.elems;
        let mut max_code: isize = -1;
        self.heap_len = 0;
        self.heap_max = HEAP_SIZE;

        for n in 0..elems {
            if tree[n].fc != 0 {
                self.heap_len += 1;
                self.heap[self.heap_len] = n as u16;
                max_code = n as isize;
                self.depth[n] = 0;
            } else {
                tree[n].dl = 0;
            }
        }

        // At least two codes of nonzero frequency.
        while self.heap_len < 2 {
            let node = if max_code < 2 {
                max_code += 1;
                max_code as usize
            } else {
                0
            };
            self.heap_len += 1;
            self.heap[self.heap_len] = node as u16;
            tree[node].fc = 1;
            self.depth[node] = 0;
            self.opt_len = self.opt_len.wrapping_sub(1);
            if let Some(stree) = desc.static_tree {
                self.static_len = self.static_len.wrapping_sub(stree[node].dl as u64);
            }
        }
        let max_code = max_code as usize;

        for n in (1..=self.heap_len / 2).rev() {
            self.pqdownheap(tree, n);
        }

        let mut node = elems;
        loop {
            // pqremove
            let n = self.heap[1] as usize;
            self.heap[1] = self.heap[self.heap_len];
            self.heap_len -= 1;
            self.pqdownheap(tree, 1);
            let m = self.heap[1] as usize;

            self.heap_max -= 1;
            self.heap[self.heap_max] = n as u16;
            self.heap_max -= 1;
            self.heap[self.heap_max] = m as u16;

            tree[node].fc = tree[n].fc.wrapping_add(tree[m].fc);
            self.depth[node] = self.depth[n].max(self.depth[m]).wrapping_add(1);
            tree[n].dl = node as u16;
            tree[m].dl = node as u16;

            self.heap[1] = node as u16;
            node += 1;
            self.pqdownheap(tree, 1);
            if self.heap_len < 2 {
                break;
            }
        }
        self.heap_max -= 1;
        self.heap[self.heap_max] = self.heap[1];

        self.gen_bitlen(tree, max_code, desc);
        gen_codes(tree, max_code, &self.bl_count);
        max_code
    }
}

fn gen_codes(tree: &mut [Ct], max_code: usize, bl_count: &[u16; MAX_BITS + 1]) {
    let mut next_code = [0u16; MAX_BITS + 1];
    let mut code = 0u32;
    for bits in 1..=MAX_BITS {
        code = (code + bl_count[bits - 1] as u32) << 1;
        next_code[bits] = code as u16;
    }
    for n in 0..=max_code {
        let len = tree[n].dl as usize;
        if len == 0 {
            continue;
        }
        tree[n].fc = bi_reverse(next_code[len] as u32, len as u32);
        next_code[len] = next_code[len].wrapping_add(1);
    }
}

/// The literal/length and distance trees, the bit length tree and the
/// symbols of the block being collected.
pub(crate) struct Trees {
    dyn_ltree: [Ct; HEAP_SIZE],
    dyn_dtree: [Ct; 2 * D_CODES + 1],
    bl_tree: [Ct; 2 * BL_CODES + 1],
    heap: Heap,
    /// Distance of each symbol (0 for a literal); zlib's `d_buf` under `LIT_MEM`.
    d_buf: [u16; LIT_BUFSIZE],
    /// Literal byte, or match length - 3; zlib's `l_buf`.
    l_buf: [u8; LIT_BUFSIZE],
    sym_next: usize,
}

impl Trees {
    pub(crate) fn new_boxed() -> Box<Self> {
        Box::new(Self {
            dyn_ltree: [Ct::default(); HEAP_SIZE],
            dyn_dtree: [Ct::default(); 2 * D_CODES + 1],
            bl_tree: [Ct::default(); 2 * BL_CODES + 1],
            heap: Heap {
                heap: [0; HEAP_SIZE],
                heap_len: 0,
                heap_max: 0,
                depth: [0; HEAP_SIZE],
                bl_count: [0; MAX_BITS + 1],
                opt_len: 0,
                static_len: 0,
            },
            d_buf: [0; LIT_BUFSIZE],
            l_buf: [0; LIT_BUFSIZE],
            sym_next: 0,
        })
    }

    /// `_tr_init` (the bit buffer lives in [`BitWriter`]).
    pub(crate) fn init(&mut self) {
        self.init_block();
    }

    fn init_block(&mut self) {
        for t in &mut self.dyn_ltree[..L_CODES] {
            t.fc = 0;
        }
        for t in &mut self.dyn_dtree[..D_CODES] {
            t.fc = 0;
        }
        for t in &mut self.bl_tree[..BL_CODES] {
            t.fc = 0;
        }
        self.dyn_ltree[END_BLOCK].fc = 1;
        self.heap.opt_len = 0;
        self.heap.static_len = 0;
        self.sym_next = 0;
    }

    /// `_tr_tally_lit`; true when the block must be flushed.
    #[inline(always)]
    pub(crate) fn tally_lit(&mut self, c: u8) -> bool {
        let i = self.sym_next & (LIT_BUFSIZE - 1);
        self.d_buf[i] = 0;
        self.l_buf[i] = c;
        self.sym_next += 1;
        self.dyn_ltree[c as usize].fc = self.dyn_ltree[c as usize].fc.wrapping_add(1);
        self.sym_next == SYM_END
    }

    /// `_tr_tally_dist`: `dist` is the match distance, `len` its length - 3.
    #[inline(always)]
    pub(crate) fn tally_dist(&mut self, dist: usize, len: usize) -> bool {
        let i = self.sym_next & (LIT_BUFSIZE - 1);
        self.d_buf[i] = dist as u16;
        self.l_buf[i] = len as u8;
        self.sym_next += 1;
        let lc = STATIC.length_code[len & 255] as usize + LITERALS + 1;
        self.dyn_ltree[lc].fc = self.dyn_ltree[lc].fc.wrapping_add(1);
        let dc = d_code((dist as u16).wrapping_sub(1) as usize);
        self.dyn_dtree[dc].fc = self.dyn_dtree[dc].fc.wrapping_add(1);
        self.sym_next == SYM_END
    }

    fn scan_tree(bl_tree: &mut [Ct], tree: &mut [Ct], max_code: usize) {
        let mut prevlen: i32 = -1;
        let mut nextlen = tree[0].dl as i32;
        let mut count = 0;
        let (mut max_count, mut min_count) = if nextlen == 0 { (138, 3) } else { (7, 4) };
        tree[max_code + 1].dl = 0xffff; // guard

        for n in 0..=max_code {
            let curlen = nextlen;
            nextlen = tree[n + 1].dl as i32;
            count += 1;
            if count < max_count && curlen == nextlen {
                continue;
            } else if count < min_count {
                let t = &mut bl_tree[curlen as usize];
                t.fc = t.fc.wrapping_add(count as u16);
            } else if curlen != 0 {
                if curlen != prevlen {
                    bl_tree[curlen as usize].fc = bl_tree[curlen as usize].fc.wrapping_add(1);
                }
                bl_tree[REP_3_6].fc = bl_tree[REP_3_6].fc.wrapping_add(1);
            } else if count <= 10 {
                bl_tree[REPZ_3_10].fc = bl_tree[REPZ_3_10].fc.wrapping_add(1);
            } else {
                bl_tree[REPZ_11_138].fc = bl_tree[REPZ_11_138].fc.wrapping_add(1);
            }
            count = 0;
            prevlen = curlen;
            (max_count, min_count) = if nextlen == 0 {
                (138, 3)
            } else if curlen == nextlen {
                (6, 3)
            } else {
                (7, 4)
            };
        }
    }

    fn send_tree(bl_tree: &[Ct], tree: &[Ct], max_code: usize, w: &mut BitWriter) {
        let mut prevlen: i32 = -1;
        let mut nextlen = tree[0].dl as i32;
        let mut count = 0;
        let (mut max_count, mut min_count) = if nextlen == 0 { (138, 3) } else { (7, 4) };
        let send_code =
            |w: &mut BitWriter, c: usize| w.send(bl_tree[c].fc as u64, bl_tree[c].dl as u32);

        for n in 0..=max_code {
            let curlen = nextlen;
            nextlen = tree[n + 1].dl as i32;
            count += 1;
            if count < max_count && curlen == nextlen {
                continue;
            } else if count < min_count {
                while count != 0 {
                    send_code(w, curlen as usize);
                    count -= 1;
                }
            } else if curlen != 0 {
                if curlen != prevlen {
                    send_code(w, curlen as usize);
                    count -= 1;
                }
                send_code(w, REP_3_6);
                w.send((count - 3) as u64, 2);
            } else if count <= 10 {
                send_code(w, REPZ_3_10);
                w.send((count - 3) as u64, 3);
            } else {
                send_code(w, REPZ_11_138);
                w.send((count - 11) as u64, 7);
            }
            count = 0;
            prevlen = curlen;
            (max_count, min_count) = if nextlen == 0 {
                (138, 3)
            } else if curlen == nextlen {
                (6, 3)
            } else {
                (7, 4)
            };
        }
    }

    /// `build_bl_tree`: returns the index in `BL_ORDER` of the last bit length code to send.
    fn build_bl_tree(&mut self, l_max: usize, d_max: usize) -> usize {
        Self::scan_tree(&mut self.bl_tree, &mut self.dyn_ltree, l_max);
        Self::scan_tree(&mut self.bl_tree, &mut self.dyn_dtree, d_max);
        self.heap.build_tree(&mut self.bl_tree, &BL_DESC);

        let mut max_blindex = BL_CODES - 1;
        while max_blindex >= 3 {
            if self.bl_tree[BL_ORDER[max_blindex] as usize].dl != 0 {
                break;
            }
            max_blindex -= 1;
        }
        self.heap.opt_len = self
            .heap
            .opt_len
            .wrapping_add(3 * (max_blindex as u64 + 1) + 5 + 5 + 4);
        max_blindex
    }

    fn send_all_trees(&self, lcodes: usize, dcodes: usize, blcodes: usize, w: &mut BitWriter) {
        w.send((lcodes - 257) as u64, 5);
        w.send((dcodes - 1) as u64, 5);
        w.send((blcodes - 4) as u64, 4);
        for &order in &BL_ORDER[..blcodes] {
            w.send(self.bl_tree[order as usize].dl as u64, 3);
        }
        Self::send_tree(&self.bl_tree, &self.dyn_ltree, lcodes - 1, w);
        Self::send_tree(&self.bl_tree, &self.dyn_dtree, dcodes - 1, w);
    }

    fn compress_block(&self, ltree: &[Ct], dtree: &[Ct], w: &mut BitWriter) {
        let n = self.sym_next;
        for (&dist, &lc) in self.d_buf[..n].iter().zip(&self.l_buf[..n]) {
            if dist == 0 {
                let t = ltree[lc as usize];
                w.send(t.fc as u64, t.dl as u32);
            } else {
                let lc = lc as usize;
                let code = STATIC.length_code[lc] as usize;
                let t = ltree[code + LITERALS + 1];
                let extra = EXTRA_LBITS[code] as u32;
                let bits =
                    t.fc as u64 | (((lc - STATIC.base_length[code] as usize) as u64) << t.dl);
                w.send(bits, t.dl as u32 + extra);

                let dist = dist as usize - 1;
                let code = d_code(dist);
                let t = dtree[code];
                let extra = EXTRA_DBITS[code] as u32;
                let bits =
                    t.fc as u64 | (((dist - STATIC.base_dist[code] as usize) as u64) << t.dl);
                w.send(bits, t.dl as u32 + extra);
            }
        }
        let t = ltree[END_BLOCK];
        w.send(t.fc as u64, t.dl as u32);
    }

    /// `_tr_stored_block`.
    pub(crate) fn stored_block(buf: &[u8], last: bool, w: &mut BitWriter) {
        w.send((STORED_BLOCK << 1) as u64 + last as u64, 3);
        w.windup();
        let len = buf.len() as u16;
        w.put_bytes(&len.to_le_bytes());
        w.put_bytes(&(!len).to_le_bytes());
        w.put_bytes(buf);
    }

    /// `_tr_flush_block`: picks stored, static or dynamic for the collected
    /// symbols and writes the block. `buf` is the block's input when it is
    /// still in the window (zlib's `block_start >= 0`), `stored_len` its length.
    pub(crate) fn flush_block(
        &mut self,
        buf: Option<&[u8]>,
        stored_len: u64,
        last: bool,
        w: &mut BitWriter,
    ) {
        let l_max = self.heap.build_tree(&mut self.dyn_ltree, &L_DESC);
        let d_max = self.heap.build_tree(&mut self.dyn_dtree, &D_DESC);
        let max_blindex = self.build_bl_tree(l_max, d_max);

        let mut opt_lenb = self.heap.opt_len.wrapping_add(3 + 7) >> 3;
        let static_lenb = self.heap.static_len.wrapping_add(3 + 7) >> 3;
        if static_lenb <= opt_lenb {
            opt_lenb = static_lenb;
        }

        match buf {
            Some(buf) if stored_len + 4 <= opt_lenb => Self::stored_block(buf, last, w),
            _ if static_lenb == opt_lenb => {
                w.send((STATIC_TREES << 1) as u64 + last as u64, 3);
                self.compress_block(&STATIC.ltree, &STATIC.dtree, w);
            }
            _ => {
                w.send((DYN_TREES << 1) as u64 + last as u64, 3);
                self.send_all_trees(l_max + 1, d_max + 1, max_blindex + 1, w);
                self.compress_block(&self.dyn_ltree, &self.dyn_dtree, w);
            }
        }
        self.init_block();
        if last {
            w.windup();
        }
    }
}

/// Bytes of state the trees hold, for the kit's table-bytes figure.
pub(crate) const TREES_BYTES: usize = std::mem::size_of::<Trees>();
