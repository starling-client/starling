//! Starling's part of java.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// A Java runtime the program running the preparer supplies (Starling on Android, where no Java
/// is installed and programs run only from the app's own library folder): its `java` and
/// `javac`, tried before any other, and the environment every Java command gets.
pub struct Supplied {
    pub java: PathBuf,
    pub javac: PathBuf,
    pub env: Vec<(String, String)>,
}

pub(super) static SUPPLIED: std::sync::OnceLock<Supplied> = std::sync::OnceLock::new();

/// Prepare with `runtime` in this process; false when one was supplied already.
pub fn supply(runtime: Supplied) -> bool {
    SUPPLIED.set(runtime).is_ok()
}

pub(crate) const SMALL_ANDROID: bool =
    cfg!(all(target_os = "android", target_pointer_width = "32"));

/// 32-bit Android cannot reserve multi-gigabyte heaps. This is a maximum, not
/// committed memory; smaller exporter requests keep their original limit.
pub(crate) fn heap_arg(heap: &str) -> String {
    heap_for(heap, max_heap_mib())
}

/// Maximum exporter heap on Android 32-bit; other targets retain their requests.
/// Below 1.5 GiB, leave more RAM for Android and the installer's renderer.
pub fn max_heap_mib() -> Option<u64> {
    if !SMALL_ANDROID {
        return None;
    }
    static LIMIT: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    Some(
        *LIMIT.get_or_init(|| {
            small_heap_limit(&fs::read_to_string("/proc/meminfo").unwrap_or_default())
        }),
    )
}

pub(super) fn small_heap_limit(meminfo: &str) -> u64 {
    let total_kib = meminfo.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        if fields.next()? != "MemTotal:" {
            return None;
        }
        let kib = fields.next()?.parse::<u64>().ok()?;
        (fields.next()? == "kB").then_some(kib)
    });
    if total_kib.is_some_and(|kib| kib >= 1536 * 1024) {
        512
    } else {
        384
    }
}

pub(super) fn heap_for(heap: &str, limit: Option<u64>) -> String {
    let bytes = heap
        .strip_suffix(['G', 'g'])
        .and_then(|n| n.parse::<u64>().ok())
        .and_then(|n| n.checked_mul(1024 * 1024 * 1024))
        .or_else(|| {
            heap.strip_suffix(['M', 'm'])
                .and_then(|n| n.parse::<u64>().ok())
                .and_then(|n| n.checked_mul(1024 * 1024))
        });
    if let Some(cap) = limit.filter(|cap| bytes.is_none_or(|n| n > cap * 1024 * 1024)) {
        format!("-Xmx{cap}M")
    } else {
        format!("-Xmx{heap}")
    }
}
