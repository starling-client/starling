//! Whether a JPEG screenshot holds every scan of its picture in full.
//!
//! zune-jpeg recovers a truncated scan even in strict mode: once a scan's entropy-coded data
//! runs into a marker, it reads zero bits for the blocks still missing, and its public API does
//! not say that it did. A screenshot cut short (and closed with an end-of-image marker) would
//! then decode, partly grey. So the scans are read here as well, from ITU T.81, without
//! computing a pixel:
//!
//! - They must cover the picture (G.1.1.1): in a sequential frame every component is in a
//!   scan; in a progressive one every component's DC coefficient and its AC coefficients 1 to
//!   63 each reach full precision (a scan with `Al` = 0).
//! - Every block of every scan must be in the scan's own data. The Huffman codes of each block
//!   are walked (F.2.2 for sequential scans, G.1.2 for progressive ones), restart interval by
//!   restart interval; a block that would need bits past the marker ending its interval means
//!   the scan was cut short.
//!
//! Only Huffman-coded frames are read: the decoder refuses arithmetic coding, lossless and
//! hierarchical frames before this runs.

const CUT_SHORT: &str = "a scan ends before its last block";

/// Checks that `bytes`, a JPEG the decoder has read, has scans covering every coefficient of
/// every component, and that each holds all of its blocks.
pub(super) fn check_complete(bytes: &[u8]) -> Result<(), String> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err("no start-of-image marker".into());
    }
    let mut frame: Option<Frame> = None;
    let mut tables = Tables::default();
    let mut restart_interval = 0;
    let mut at = 2;
    loop {
        if bytes.get(at) != Some(&0xff) {
            return Err(format!("no marker at byte {at}"));
        }
        // Any number of fill bytes may come before a marker (B.1.1.2).
        while bytes.get(at + 1) == Some(&0xff) {
            at += 1;
        }
        let Some(&marker) = bytes.get(at + 1) else {
            return Err("no end-of-image marker".into());
        };
        at += 2;
        match marker {
            0xd9 => {
                if at != bytes.len() {
                    return Err("data after the end-of-image marker".into());
                }
                break;
            }
            0x01 => continue,
            0xd0..=0xd8 => return Err(format!("marker {marker:#04x} outside a scan")),
            _ => {}
        }
        let length = bytes
            .get(at..at + 2)
            .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
            .filter(|&length| length >= 2)
            .ok_or_else(|| format!("marker {marker:#04x} has no length"))?;
        let segment = bytes
            .get(at + 2..at + length)
            .ok_or_else(|| format!("marker {marker:#04x} runs past the file"))?;
        at += length;
        match marker {
            // SOF0 to SOF15, less DHT (C4), JPG (C8) and DAC (CC).
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                if frame.is_some() {
                    return Err("more than one frame".into());
                }
                frame = Some(Frame::read(marker, segment)?);
            }
            0xc4 => tables.read(segment)?,
            0xdd => {
                let interval = segment.get(..2).filter(|_| segment.len() == 2);
                let interval = interval.ok_or("a restart interval of the wrong length")?;
                restart_interval = usize::from(u16::from_be_bytes([interval[0], interval[1]]));
            }
            0xda => {
                let frame = frame.as_mut().ok_or("a scan before the frame header")?;
                let scan = frame.scan_header(segment, &tables)?;
                let intervals = entropy_coded_segments(bytes, at)?;
                at = intervals.last().map_or(at, |&(_, end)| end);
                frame.walk(&scan, &intervals, bytes, &tables, restart_interval)?;
                frame.cover(&scan);
            }
            _ => {}
        }
    }
    frame.ok_or("no frame header")?.covered()
}

/// The entropy-coded segments of the scan whose data starts at `at`: one per restart interval,
/// each from its first byte to the marker after it (a restart marker, or the marker ending the
/// scan).
fn entropy_coded_segments(bytes: &[u8], mut at: usize) -> Result<Vec<(usize, usize)>, String> {
    let mut segments = Vec::new();
    let mut start = at;
    loop {
        let found = bytes
            .get(at..)
            .and_then(|rest| rest.iter().position(|&b| b == 0xff));
        let Some(found) = found else {
            return Err("a scan with no marker after it".into());
        };
        at += found;
        match bytes.get(at + 1) {
            // A stuffed data byte.
            Some(0x00) => at += 2,
            Some(0xd0..=0xd7) => {
                segments.push((start, at));
                at += 2;
                start = at;
            }
            Some(_) => {
                segments.push((start, at));
                return Ok(segments);
            }
            None => return Err("a scan with no marker after it".into()),
        }
    }
}

/// The Huffman tables defined so far (B.2.4.2), by class and destination.
#[derive(Default)]
struct Tables {
    dc: [Option<Huffman>; 4],
    ac: [Option<Huffman>; 4],
}

impl Tables {
    fn read(&mut self, mut segment: &[u8]) -> Result<(), String> {
        while let [class_and_id, rest @ ..] = segment {
            let counts: [u8; 16] = rest
                .get(..16)
                .and_then(|counts| counts.try_into().ok())
                .ok_or("a short Huffman table")?;
            let total = counts.iter().map(|&n| usize::from(n)).sum::<usize>();
            let values = rest.get(16..16 + total).ok_or("a short Huffman table")?;
            let table = Some(Huffman::new(&counts, values)?);
            let id = usize::from(class_and_id & 0x0f);
            match (class_and_id >> 4, id) {
                (0, 0..=3) => self.dc[id] = table,
                (1, 0..=3) => self.ac[id] = table,
                _ => return Err(format!("a Huffman table {class_and_id:#04x}")),
            }
            segment = &rest[16 + total..];
        }
        Ok(())
    }
}

/// A canonical Huffman table, decoded as in F.2.2.3.
struct Huffman {
    /// For each code length 1 to 16: the largest code (`-1` when none), the first code and
    /// the index of its value.
    max_code: [i32; 17],
    min_code: [i32; 17],
    first_value: [i32; 17],
    values: Vec<u8>,
}

impl Huffman {
    fn new(counts: &[u8; 16], values: &[u8]) -> Result<Self, String> {
        let mut table = Huffman {
            max_code: [-1; 17],
            min_code: [0; 17],
            first_value: [0; 17],
            values: values.to_vec(),
        };
        let (mut code, mut index) = (0i32, 0i32);
        for length in 1..=16 {
            let count = i32::from(counts[length - 1]);
            table.first_value[length] = index;
            table.min_code[length] = code;
            code += count;
            index += count;
            if count > 0 {
                table.max_code[length] = code - 1;
            }
            if code > 1 << length {
                return Err("a Huffman table with more codes than fit".into());
            }
            code <<= 1;
        }
        Ok(table)
    }

    fn decode(&self, bits: &mut Bits<'_>) -> Result<u8, String> {
        let mut code = 0;
        for length in 1..=16 {
            code = code << 1 | bits.bit()? as i32;
            if code <= self.max_code[length] {
                let index = self.first_value[length] + code - self.min_code[length];
                return Ok(self.values[index as usize]);
            }
        }
        Err("an invalid Huffman code".into())
    }
}

/// The bits of one entropy-coded segment, `FF 00` read as `FF` (F.1.2.3). Running out of them
/// means the segment was cut short.
struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
    end: usize,
    byte: u8,
    left: u32,
}

impl<'a> Bits<'a> {
    fn new(bytes: &'a [u8], (start, end): (usize, usize)) -> Self {
        Bits {
            bytes,
            at: start,
            end,
            byte: 0,
            left: 0,
        }
    }

    fn bit(&mut self) -> Result<u32, String> {
        if self.left == 0 {
            if self.at >= self.end {
                return Err(CUT_SHORT.into());
            }
            self.byte = self.bytes[self.at];
            // Inside a segment every `FF` is followed by its stuffed `00`.
            self.at += if self.byte == 0xff { 2 } else { 1 };
            self.left = 8;
        }
        self.left -= 1;
        Ok(u32::from(self.byte >> self.left) & 1)
    }

    fn skip(&mut self, count: u32) -> Result<u32, String> {
        (0..count).try_fold(0, |value, _| Ok(value << 1 | self.bit()?))
    }
}

/// A frame header's components, its block grid, and what its scans have covered so far.
struct Frame {
    progressive: bool,
    width: usize,
    height: usize,
    max_h: usize,
    max_v: usize,
    components: Vec<Component>,
}

struct Component {
    id: u8,
    h: usize,
    v: usize,
    /// Sequential: the component is in a scan. Progressive: its DC coefficient is at full
    /// precision.
    dc: bool,
    /// Progressive: bit `k` is set once AC coefficient `k` (1 to 63) is at full precision.
    ac: u64,
    /// Progressive: for each block of the MCU-padded grid, bit `k` is set once coefficient `k`
    /// is non-zero, which decides the correction bits of a refinement scan (G.1.2.3).
    non_zero: Vec<u64>,
}

/// Bits 1 to 63: every AC coefficient.
const ALL_AC: u64 = !1;

/// One scan header (B.2.3): its components (the index in the frame, then the DC and AC table
/// destinations), spectral selection and successive approximation.
struct Scan {
    components: Vec<(usize, usize, usize)>,
    start: usize,
    end: usize,
    high: u8,
    low: u8,
}

impl Frame {
    fn read(marker: u8, segment: &[u8]) -> Result<Self, String> {
        // P, Y, X, Nf, then Nf times (C, H/V, Tq) (B.2.2).
        let [_, y0, y1, x0, x1, count, ref fields @ ..] = *segment else {
            return Err("a short frame header".into());
        };
        if count == 0 || fields.len() != 3 * usize::from(count) {
            return Err("a frame header whose length does not match its components".into());
        }
        let (width, height) = (
            usize::from(u16::from_be_bytes([x0, x1])),
            usize::from(u16::from_be_bytes([y0, y1])),
        );
        if width == 0 || height == 0 {
            return Err("a frame with no size".into());
        }
        let components = fields
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| {
                let (h, v) = (usize::from(c[1] >> 4), usize::from(c[1] & 0x0f));
                if !(1..=4).contains(&h) || !(1..=4).contains(&v) {
                    return Err(format!("component {} has sampling {h}x{v}", c[0]));
                }
                Ok(Component {
                    id: c[0],
                    h,
                    v,
                    dc: false,
                    ac: 0,
                    non_zero: Vec::new(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Frame {
            progressive: matches!(marker, 0xc2 | 0xc6 | 0xca | 0xce),
            width,
            height,
            max_h: components.iter().map(|c| c.h).max().unwrap_or(1),
            max_v: components.iter().map(|c| c.v).max().unwrap_or(1),
            components,
        })
    }

    /// MCUs across and down the frame when its scans are interleaved (A.2.3).
    fn mcus(&self) -> (usize, usize) {
        (
            self.width.div_ceil(8 * self.max_h),
            self.height.div_ceil(8 * self.max_v),
        )
    }

    fn scan_header(&self, segment: &[u8], tables: &Tables) -> Result<Scan, String> {
        // Ns, then Ns times (Cs, Td/Ta), then Ss, Se, Ah/Al.
        let count = usize::from(*segment.first().ok_or("a short scan header")?);
        if count == 0 || segment.len() != 4 + 2 * count {
            return Err("a scan header whose length does not match its components".into());
        }
        let (start, end) = (segment[1 + 2 * count], segment[2 + 2 * count]);
        let approximation = segment[3 + 2 * count];
        if start > end || end > 63 || (self.progressive && start == 0 && end != 0) {
            return Err(format!("a scan of coefficients {start} to {end}"));
        }
        let components = segment[1..1 + 2 * count]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|selector| {
                let index = self
                    .components
                    .iter()
                    .position(|c| c.id == selector[0])
                    .ok_or_else(|| {
                        format!("a scan of component {}, not in the frame", selector[0])
                    })?;
                let (dc, ac) = (
                    usize::from(selector[1] >> 4),
                    usize::from(selector[1] & 0x0f),
                );
                // Td and Ta name one of four table destinations each (B.2.3), whether or not
                // the scan uses that class.
                if dc > 3 || ac > 3 {
                    return Err(format!(
                        "a scan naming Huffman table destinations {dc} and {ac}, past 3"
                    ));
                }
                let uses_dc = !self.progressive || (start == 0 && approximation >> 4 == 0);
                let uses_ac = !self.progressive || start > 0;
                if uses_dc && tables.dc.get(dc).is_none_or(Option::is_none)
                    || uses_ac && tables.ac.get(ac).is_none_or(Option::is_none)
                {
                    return Err("a scan using a Huffman table that is not defined".into());
                }
                Ok((index, dc, ac))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Scan {
            components,
            start: usize::from(start),
            end: usize::from(end),
            high: approximation >> 4,
            low: approximation & 0x0f,
        })
    }

    /// Reads every block of `scan` from its `intervals` (A.2.2, A.2.3, B.2.5).
    fn walk(
        &mut self,
        scan: &Scan,
        intervals: &[(usize, usize)],
        bytes: &[u8],
        tables: &Tables,
        restart_interval: usize,
    ) -> Result<(), String> {
        let (mcus_across, mcus_down) = self.mcus();
        if self.progressive {
            for &(index, ..) in &scan.components {
                let c = &mut self.components[index];
                if c.non_zero.is_empty() {
                    c.non_zero = vec![0; mcus_across * c.h * mcus_down * c.v];
                }
            }
        }
        // A scan of one component is not interleaved: its units are that component's own
        // blocks, without the MCU padding.
        let (across, units) = if let [(index, ..)] = scan.components[..] {
            let c = &self.components[index];
            let across = (self.width * c.h).div_ceil(self.max_h).div_ceil(8);
            let down = (self.height * c.v).div_ceil(self.max_v).div_ceil(8);
            (across, across * down)
        } else {
            (mcus_across, mcus_across * mcus_down)
        };
        let per_interval = if restart_interval == 0 {
            units
        } else {
            restart_interval
        };
        let progressive = self.progressive;
        let mut intervals = intervals.iter();
        let mut unit = 0;
        while unit < units {
            let interval = intervals.next().ok_or(CUT_SHORT)?;
            let mut bits = Bits::new(bytes, *interval);
            let mut end_of_band_run = 0;
            for _ in 0..per_interval.min(units - unit) {
                let (x, y) = (unit % across, unit / across);
                for &(index, dc, ac) in &scan.components {
                    let c = &mut self.components[index];
                    let (h, v) = if scan.components.len() == 1 {
                        (1, 1)
                    } else {
                        (c.h, c.v)
                    };
                    let row = mcus_across * c.h;
                    for dy in 0..v {
                        for dx in 0..h {
                            let block = (y * v + dy) * row + x * h + dx;
                            let non_zero = c.non_zero.get_mut(block);
                            let dc = tables.dc.get(dc).and_then(Option::as_ref);
                            let ac = tables.ac.get(ac).and_then(Option::as_ref);
                            let block = Block {
                                bits: &mut bits,
                                dc,
                                ac,
                                non_zero,
                                end_of_band_run: &mut end_of_band_run,
                            };
                            if progressive {
                                block.progressive(scan)?;
                            } else {
                                block.sequential()?;
                            }
                        }
                    }
                }
                unit += 1;
            }
        }
        Ok(())
    }

    /// Records what `scan` covered.
    fn cover(&mut self, scan: &Scan) {
        let ac = (scan.start.max(1)..=scan.end).fold(0u64, |bits, k| bits | 1 << k);
        for &(index, ..) in &scan.components {
            let c = &mut self.components[index];
            if !self.progressive {
                c.dc = true;
            } else if scan.low == 0 {
                c.dc |= scan.start == 0;
                c.ac |= ac;
            }
        }
    }

    fn covered(&self) -> Result<(), String> {
        for c in &self.components {
            if !c.dc {
                return Err(format!("no full scan of component {}", c.id));
            }
            if self.progressive && c.ac != ALL_AC {
                let missing = (1..64).find(|k| c.ac & 1 << k == 0).unwrap_or(0);
                return Err(format!(
                    "no full scan of component {}'s coefficient {missing}",
                    c.id
                ));
            }
        }
        Ok(())
    }
}

/// One block's codes in a scan.
struct Block<'b, 'a> {
    bits: &'b mut Bits<'a>,
    dc: Option<&'b Huffman>,
    ac: Option<&'b Huffman>,
    non_zero: Option<&'b mut u64>,
    end_of_band_run: &'b mut u32,
}

impl Block<'_, '_> {
    fn table(table: Option<&Huffman>) -> Result<&Huffman, String> {
        table.ok_or_else(|| "a scan using a Huffman table that is not defined".into())
    }

    /// F.2.2.1 and F.2.2.2: the DC difference, then the AC codes up to the end of block.
    fn sequential(self) -> Result<(), String> {
        let size = Self::table(self.dc)?.decode(self.bits)?;
        self.bits.skip(u32::from(size))?;
        let ac = Self::table(self.ac)?;
        let mut k = 1;
        while k < 64 {
            let code = ac.decode(self.bits)?;
            let (run, size) = (usize::from(code >> 4), u32::from(code & 0x0f));
            if size == 0 {
                if run != 15 {
                    break;
                }
                k += 16;
                continue;
            }
            k += run;
            if k > 63 {
                return Err("a block with more than 64 coefficients".into());
            }
            self.bits.skip(size)?;
            k += 1;
        }
        Ok(())
    }

    /// G.1.2.1 to G.1.2.3.
    fn progressive(self, scan: &Scan) -> Result<(), String> {
        let Block {
            bits,
            dc,
            ac,
            non_zero,
            end_of_band_run,
        } = self;
        if scan.start == 0 {
            return if scan.high == 0 {
                let size = Self::table(dc)?.decode(bits)?;
                bits.skip(u32::from(size)).map(drop)
            } else {
                bits.bit().map(drop)
            };
        }
        let ac = Self::table(ac)?;
        let non_zero = non_zero.ok_or("a block outside the frame")?;
        let mut k = scan.start;
        if scan.high == 0 {
            // A first scan of a band.
            if *end_of_band_run > 0 {
                *end_of_band_run -= 1;
                return Ok(());
            }
            while k <= scan.end {
                let code = ac.decode(bits)?;
                let (run, size) = (usize::from(code >> 4), u32::from(code & 0x0f));
                if size == 0 {
                    if run < 15 {
                        *end_of_band_run = (1 << run) + bits.skip(run as u32)? - 1;
                        break;
                    }
                    k += 16;
                    continue;
                }
                k += run;
                if k > scan.end {
                    return Err("a coefficient past its scan's band".into());
                }
                bits.skip(size)?;
                *non_zero |= 1 << k;
                k += 1;
            }
            return Ok(());
        }
        // A refinement scan: a correction bit for each coefficient already non-zero, a sign
        // for each that becomes non-zero.
        if *end_of_band_run == 0 {
            while k <= scan.end {
                let code = ac.decode(bits)?;
                let (mut run, size) = (i32::from(code >> 4), code & 0x0f);
                let becomes_non_zero = size != 0;
                if becomes_non_zero {
                    if size != 1 {
                        return Err("a refinement of more than one bit".into());
                    }
                    bits.bit()?;
                } else if run != 15 {
                    *end_of_band_run = (1 << run) + bits.skip(run as u32)?;
                    break;
                }
                while k <= scan.end {
                    if *non_zero & 1 << k != 0 {
                        bits.bit()?;
                    } else {
                        run -= 1;
                        if run < 0 {
                            break;
                        }
                    }
                    k += 1;
                }
                if becomes_non_zero {
                    if k > scan.end {
                        return Err("a coefficient past its scan's band".into());
                    }
                    *non_zero |= 1 << k;
                }
                k += 1;
            }
        }
        if *end_of_band_run > 0 {
            while k <= scan.end {
                if *non_zero & 1 << k != 0 {
                    bits.bit()?;
                }
                k += 1;
            }
            *end_of_band_run -= 1;
        }
        Ok(())
    }
}
