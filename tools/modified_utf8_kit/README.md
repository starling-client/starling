# modified UTF-8 kit

Times `sunrise_core::modified_utf8` (`crates/sunrise-core/src/modified_utf8.rs`) against HotSpot's
`DataOutputStream.writeUTF` and `DataInputStream.readUTF`, the JDK's modified UTF-8, with the method of Isloka's math
kits. Bit parity is the job of `tools/test_modified_utf8_parity.py`; this kit only checks that its copies agree.

## Run

Needs Linux on x86-64, Rust, Python 3, `nm` (binutils) and a JDK 25 (`java` and `javac` on `PATH`, or `--java`).

```sh
python3 tools/modified_utf8_kit/run.py                       # every row: encode and decode, 12 input sets
python3 tools/modified_utf8_kit/run.py --rows "decode cjk"   # rows whose "op set" starts with one of these
```

It builds in `.local/modified-utf8-kit/` and writes `report.json` there. Run it on an otherwise idle machine
(`crew run --kind bench --exclusive` here): the numbers are sensitive to load.

## What it times

Per row, one operation over one input set (`rust/src/main.rs`, `gen`): strings of 8, 64 or 1024 UTF-16 units in four
mixes, `ascii` (printable ASCII), `latin` (three units in four from U+0080 to U+07FF, two bytes each), `cjk` (three in
four from U+4E00 to U+9FFF, three bytes) and `emoji` (half surrogate pairs, half ASCII), each set about 16,384 units.

- `encode`: every string's length prefix and bytes into one buffer, as the NBT writers write a string; HotSpot
  `writeUTF` into a `DataOutputStream` over a `ByteArrayOutputStream`.
- `decode`: every string read back from those bytes into a slot of its own, as the NBT readers do; HotSpot `readUTF`
  from a `DataInputStream` over a `ByteArrayInputStream`.

A figure is the median nanoseconds per string over 400 timed batches, after 200 (Rust) or 2,000 (HotSpot) batches of
warm-up. Both sides run pinned to one core with `taskset`: on a hybrid processor a performance core (Intel's P-cores
from `/sys/devices/cpu_core/cpus`, or the cores whose `cpuinfo_max_freq` is clearly the highest), else CPU 2.

## Placement

The same code can run a few percent faster or slower depending only on where it lands in the binary. The kit builds
the module as several byte-identical copies (`--copies`, 4 by default), each with its own encode and decode loops,
and every Rust figure is the mean over the copies, with the spread between them beside it. The compiler emits a
module's functions in the order of their length-prefixed names, so a filler function named to sort just before each
copy's loops shifts that copy; the fillers before every copy but the first are the same size, so the copies are
the same distance apart, and while that puts two copies on one offset within a 64-byte line the kit lengthens the
fillers and builds again. It reads every copy's functions from the symbol table (`nm`), prints their offsets, and a
run whose copies share an offset, or were merged into one, is marked `NOT placement-averaged` and doesn't pass.

## Confidence

For each row, Rust (every copy) and HotSpot run in alternating rounds over fresh JVMs: 8 JVM starts of 5 rounds each by
default (`--confidence-starts`, `--confidence-rounds`), the JVM waiting between its rounds (`stepped`) while Rust runs
on the same core, and HotSpot going first in every other round. Each round's mean over the copies is divided by
HotSpot's round; the row reports the median of those ratios with a bootstrap 95% interval resampled by JVM start (whole starts, then rounds within them: rounds of one JVM share its state), each copy's own median
ratio, and how far HotSpot moved between JVM starts. A row is `faster` when the whole interval is under 0.98,
`slower` when it is over 1.00, and `inside the noise` otherwise.

The verdict is PASS when every row is `faster`, the copies are placement-averaged, and the encode loops make no heap
allocation (a counting allocator counts them; decode allocates each string it returns, as `readUTF` does).

## Files

- `run.py`: the driver; it writes the bench crate to `.local/modified-utf8-kit/rust/` (`copies.rs`, `copies/`)
- `rust/`: the bench program (`gen`, `check`, `bench`) and its manifest
- `mods/sunrise/src/test/java/dev/sunriselab/parity/Mutf8Bench.java`: the JVM side
