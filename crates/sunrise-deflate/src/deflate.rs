// Altered from deflate.c of zlib 1.3.2 (compress data using the deflation algorithm):
// Copyright (C) 1995-2026 Jean-loup Gailly and Mark Adler. For conditions of
// distribution and use, see the copyright notice in this crate's LICENSE
// (zlib.h's notice).
//
// This is a Rust port of the level-6 path (deflate_slow, longest_match,
// fill_window and the hash chains) for one-shot compression with Z_FINISH. It
// is not the original software.

//! The match finder and the lazy evaluation loop: `deflate.c`'s `deflate_slow`.
//!
//! Every input byte goes through the 64 KiB window as in zlib, including the
//! bytes past the end of the data that `longest_match` reads before it clips a
//! match to the lookahead: which of two candidates wins can depend on them, so
//! the window's contents (zeroed up to `high_water`, stale after a slide) are
//! kept exactly as zlib keeps them.

use crate::bits::BitWriter;
use crate::trees::Trees;

const W_BITS: usize = 15;
const W_SIZE: usize = 1 << W_BITS;
const W_MASK: usize = W_SIZE - 1;
const WINDOW_SIZE: usize = 2 * W_SIZE;
const HASH_BITS: usize = 8 + 7; // memLevel 8
const HASH_SIZE: usize = 1 << HASH_BITS;
const HASH_MASK: usize = HASH_SIZE - 1;
const HASH_SHIFT: usize = HASH_BITS.div_ceil(MIN_MATCH);
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
const MIN_LOOKAHEAD: usize = MAX_MATCH + MIN_MATCH + 1;
const MAX_DIST: usize = W_SIZE - MIN_LOOKAHEAD;
const WIN_INIT: usize = MAX_MATCH;
const TOO_FAR: usize = 4096;

/// `configuration_table[6]`: reduce the search above this match length.
const GOOD_MATCH: usize = 8;
/// Do not perform a lazy search above this match length.
const MAX_LAZY_MATCH: usize = 16;
/// Stop searching at this match length.
const NICE_MATCH: usize = 128;
/// Longest hash chain searched.
const MAX_CHAIN: u32 = 128;

/// `deflate_state` for windowBits 15, memLevel 8, level 6 and the default strategy.
pub(crate) struct State {
    window: Box<[u8; WINDOW_SIZE]>,
    prev: Box<[u16; W_SIZE]>,
    head: Box<[u16; HASH_SIZE]>,
    trees: Box<Trees>,

    strstart: usize,
    block_start: isize,
    lookahead: usize,
    insert: usize,
    ins_h: usize,
    match_length: usize,
    prev_length: usize,
    /// Where the match found at `strstart` starts. A slide moves it back by
    /// `W_SIZE` with wrapping, so a pending match at the full distance can sit
    /// at -1 (`usize::MAX`), as zlib's unsigned `match_start` does; distances
    /// from it are taken with `wrapping_sub`.
    match_start: usize,
    prev_match: usize,
    match_available: bool,
    high_water: usize,
    next_in: usize,
}

/// Bytes the encoder holds: window, hash chains, trees and symbol buffer.
pub(crate) const STATE_BYTES: usize = WINDOW_SIZE
    + 2 * W_SIZE
    + 2 * HASH_SIZE
    + crate::trees::TREES_BYTES
    + std::mem::size_of::<State>();

fn boxed<T: Copy + Default, const N: usize>() -> Box<[T; N]> {
    vec![T::default(); N]
        .into_boxed_slice()
        .try_into()
        .unwrap_or_else(|_| unreachable!("the vector has N elements"))
}

#[inline(always)]
fn read_u16(w: &[u8; WINDOW_SIZE], at: usize) -> u16 {
    u16::from_le_bytes([w[at], w[at + 1]])
}

#[inline(always)]
fn read_u64(w: &[u8; WINDOW_SIZE], at: usize) -> u64 {
    u64::from_le_bytes(w[at..at + 8].try_into().unwrap())
}

impl State {
    pub(crate) fn new() -> Self {
        Self {
            window: boxed(),
            prev: boxed(),
            head: boxed(),
            trees: Trees::new_boxed(),
            strstart: 0,
            block_start: 0,
            lookahead: 0,
            insert: 0,
            ins_h: 0,
            match_length: MIN_MATCH - 1,
            prev_length: MIN_MATCH - 1,
            match_start: 0,
            prev_match: 0,
            match_available: false,
            high_water: 0,
            next_in: 0,
        }
    }

    /// `deflateReset` + `lm_init`, plus `high_water = 0` as a fresh `deflateInit2` has:
    /// the stream depends only on its own input, never on an earlier one.
    fn reset(&mut self) {
        self.head.fill(0);
        self.strstart = 0;
        self.block_start = 0;
        self.lookahead = 0;
        self.insert = 0;
        self.match_length = MIN_MATCH - 1;
        self.prev_length = MIN_MATCH - 1;
        self.match_available = false;
        self.ins_h = 0;
        self.match_start = 0;
        self.prev_match = 0;
        self.high_water = 0;
        self.next_in = 0;
        self.trees.init();
    }

    /// Raw deflate of all of `input`, as one `deflate(Z_FINISH)` call with
    /// unlimited output space.
    pub(crate) fn compress(&mut self, input: &[u8], w: &mut BitWriter) {
        self.reset();
        self.deflate_slow(input, w);
    }

    #[inline(always)]
    fn update_hash(h: usize, c: u8) -> usize {
        ((h << HASH_SHIFT) ^ c as usize) & HASH_MASK
    }

    /// `INSERT_STRING`: returns the previous head of the string's hash chain.
    #[inline(always)]
    fn insert_string(&mut self, s: usize) -> usize {
        self.ins_h = Self::update_hash(self.ins_h, self.window[s + MIN_MATCH - 1]);
        let head = self.head[self.ins_h];
        self.prev[s & W_MASK] = head;
        self.head[self.ins_h] = s as u16;
        head as usize
    }

    fn slide_hash(&mut self) {
        for m in self.head.iter_mut() {
            *m = m.saturating_sub(W_SIZE as u16);
        }
        for m in self.prev.iter_mut() {
            *m = m.saturating_sub(W_SIZE as u16);
        }
    }

    fn fill_window(&mut self, input: &[u8]) {
        loop {
            let mut more = WINDOW_SIZE - self.lookahead - self.strstart;

            if self.strstart >= W_SIZE + MAX_DIST {
                self.window.copy_within(W_SIZE..W_SIZE + W_SIZE - more, 0);
                self.match_start = self.match_start.wrapping_sub(W_SIZE);
                self.strstart -= W_SIZE;
                self.block_start -= W_SIZE as isize;
                if self.insert > self.strstart {
                    self.insert = self.strstart;
                }
                self.slide_hash();
                more += W_SIZE;
            }
            let avail = input.len() - self.next_in;
            if avail == 0 {
                break;
            }

            // read_buf (the checksum is taken over the whole input by the caller).
            let n = avail.min(more);
            let at = self.strstart + self.lookahead;
            self.window[at..at + n].copy_from_slice(&input[self.next_in..self.next_in + n]);
            self.next_in += n;
            self.lookahead += n;

            if self.lookahead + self.insert >= MIN_MATCH {
                let mut s = self.strstart - self.insert;
                self.ins_h = self.window[s] as usize;
                self.ins_h = Self::update_hash(self.ins_h, self.window[s + 1]);
                while self.insert != 0 {
                    self.ins_h = Self::update_hash(self.ins_h, self.window[s + MIN_MATCH - 1]);
                    self.prev[s & W_MASK] = self.head[self.ins_h];
                    self.head[self.ins_h] = s as u16;
                    s += 1;
                    self.insert -= 1;
                    if self.lookahead + self.insert < MIN_MATCH {
                        break;
                    }
                }
            }
            if !(self.lookahead < MIN_LOOKAHEAD && self.next_in != input.len()) {
                break;
            }
        }

        // Zero WIN_INIT bytes past the data the first time they are reached.
        if self.high_water < WINDOW_SIZE {
            let curr = self.strstart + self.lookahead;
            if self.high_water < curr {
                let init = (WINDOW_SIZE - curr).min(WIN_INIT);
                self.window[curr..curr + init].fill(0);
                self.high_water = curr + init;
            } else if self.high_water < curr + WIN_INIT {
                let init = (curr + WIN_INIT - self.high_water).min(WINDOW_SIZE - self.high_water);
                self.window[self.high_water..self.high_water + init].fill(0);
                self.high_water += init;
            }
        }
    }

    /// `longest_match`: the longest match for the string at `strstart` along
    /// the hash chain from `cur_match`; sets `match_start`.
    ///
    /// The length of a candidate is its common prefix with the current string,
    /// counted from byte 3 with byte 2 taken as equal (zlib skips it: the hash
    /// already fixes it), up to 258; zlib compares byte by byte and this eight
    /// at a time, which finds the same length.
    #[inline(always)]
    fn longest_match(&mut self, mut cur_match: usize) -> usize {
        let w = &*self.window;
        let prev = &*self.prev;
        let scan = self.strstart;
        let mut chain_length = MAX_CHAIN;
        let mut best_len = self.prev_length;
        let mut nice_match = NICE_MATCH;
        let limit = scan.saturating_sub(MAX_DIST);

        let scan_start = read_u16(w, scan);
        let mut scan_end = read_u16(w, scan + best_len - 1);

        if self.prev_length >= GOOD_MATCH {
            chain_length >>= 2;
        }
        if nice_match > self.lookahead {
            nice_match = self.lookahead;
        }

        loop {
            let m = cur_match;
            if read_u16(w, m + best_len - 1) == scan_end && read_u16(w, m) == scan_start {
                let mut len = MAX_MATCH;
                let mut k = 3;
                while k < MAX_MATCH {
                    let diff = read_u64(w, scan + k) ^ read_u64(w, m + k);
                    if diff != 0 {
                        len = (k + (diff.trailing_zeros() / 8) as usize).min(MAX_MATCH);
                        break;
                    }
                    k += 8;
                }
                if len > best_len {
                    self.match_start = cur_match;
                    best_len = len;
                    if len >= nice_match {
                        break;
                    }
                    scan_end = read_u16(w, scan + best_len - 1);
                }
            }
            cur_match = prev[cur_match & W_MASK] as usize;
            if cur_match <= limit {
                break;
            }
            chain_length -= 1;
            if chain_length == 0 {
                break;
            }
        }
        best_len.min(self.lookahead)
    }

    /// `FLUSH_BLOCK_ONLY`.
    fn flush_block(&mut self, last: bool, w: &mut BitWriter) {
        let stored_len = (self.strstart as isize - self.block_start) as u64;
        let buf =
            (self.block_start >= 0).then(|| &self.window[self.block_start as usize..self.strstart]);
        self.trees.flush_block(buf, stored_len, last, w);
        self.block_start = self.strstart as isize;
    }

    fn deflate_slow(&mut self, input: &[u8], w: &mut BitWriter) {
        loop {
            if self.lookahead < MIN_LOOKAHEAD {
                self.fill_window(input);
                if self.lookahead == 0 {
                    break;
                }
            }

            let mut hash_head = 0;
            if self.lookahead >= MIN_MATCH {
                hash_head = self.insert_string(self.strstart);
            }

            self.prev_length = self.match_length;
            self.prev_match = self.match_start;
            self.match_length = MIN_MATCH - 1;

            if hash_head != 0
                && self.prev_length < MAX_LAZY_MATCH
                && self.strstart.wrapping_sub(hash_head) <= MAX_DIST
            {
                self.match_length = self.longest_match(hash_head);
                if self.match_length == MIN_MATCH
                    && self.strstart.wrapping_sub(self.match_start) > TOO_FAR
                {
                    self.match_length = MIN_MATCH - 1;
                }
            }

            if self.prev_length >= MIN_MATCH && self.match_length <= self.prev_length {
                let max_insert = self.strstart + self.lookahead - MIN_MATCH;
                let bflush = self.trees.tally_dist(
                    (self.strstart - 1).wrapping_sub(self.prev_match),
                    self.prev_length - MIN_MATCH,
                );
                self.lookahead -= self.prev_length - 1;
                let mut left = self.prev_length - 2;
                loop {
                    self.strstart += 1;
                    if self.strstart <= max_insert {
                        self.insert_string(self.strstart);
                    }
                    left -= 1;
                    if left == 0 {
                        break;
                    }
                }
                self.prev_length = 0;
                self.match_available = false;
                self.match_length = MIN_MATCH - 1;
                self.strstart += 1;
                if bflush {
                    self.flush_block(false, w);
                }
            } else if self.match_available {
                let bflush = self.trees.tally_lit(self.window[self.strstart - 1]);
                if bflush {
                    self.flush_block(false, w);
                }
                self.strstart += 1;
                self.lookahead -= 1;
            } else {
                self.match_available = true;
                self.strstart += 1;
                self.lookahead -= 1;
            }
        }
        if self.match_available {
            self.trees.tally_lit(self.window[self.strstart - 1]);
            self.match_available = false;
        }
        self.insert = self.strstart.min(MIN_MATCH - 1);
        self.flush_block(true, w);
    }
}
