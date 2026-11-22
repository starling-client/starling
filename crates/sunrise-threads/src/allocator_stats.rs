//! The heap the program has in use in mimalloc, from its statistics.

/// The number after `"key":` on a line of mimalloc's JSON.
fn field(line: &str, key: &str) -> Option<i64> {
    let from = line.find(&format!("\"{key}\":"))? + key.len() + 3;
    let rest = line[from..].trim_start();
    let sign = usize::from(rest.starts_with('-'));
    let digits = rest[sign..]
        .chars()
        .take_while(char::is_ascii_digit)
        .count();
    rest[..sign + digits].parse().ok()
}

/// Bytes of the pages `page_bins` has in use, plus the huge blocks `malloc_huge` counts by size.
/// `malloc_bins` counts blocks, not pages, and only in builds with `MI_STAT` > 1.
fn in_use(json: &str) -> Option<usize> {
    let huge = json.lines().find(|l| l.contains("\"malloc_huge\":"))?;
    let bins = json.split("\"page_bins\"").nth(1)?;
    let bins = bins.split(']').next()?;
    let mut total = field(huge, "current")?;
    for line in bins.lines().filter(|l| l.contains("\"block_size\"")) {
        total += field(line, "current")? * field(line, "page_size")?;
    }
    // A block freed on another thread can be counted out before its allocating thread's count is merged in.
    Some(usize::try_from(total).unwrap_or(0))
}

fn stats_json() -> Option<String> {
    // SAFETY: a null buffer asks mimalloc for one of its own, which `take_json` frees.
    take_json(unsafe { libmimalloc_sys::mi_stats_get_json(0, std::ptr::null_mut()) }.cast())
}

/// The text of a JSON buffer mimalloc allocated for a stats call given no buffer, which this frees.
fn take_json(raw: *mut std::ffi::c_char) -> Option<String> {
    if raw.is_null() {
        return None;
    }
    // SAFETY: mimalloc returns a NUL-terminated string.
    let text = unsafe { std::ffi::CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: `raw` came from a mimalloc stats call with no buffer given.
    unsafe { libmimalloc_sys::mi_free(raw.cast()) };
    Some(text)
}

/// The bytes of the pages holding allocations now: the heap in use, counting each page whole.
pub fn live_bytes() -> Option<usize> {
    in_use(&stats_json()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_pages_in_use_per_bin() {
        let json = r#"{
  "malloc_huge": { "total": 9, "peak": 9, "current": 1000 },
  "malloc_bins": [
    { "total": 900, "peak": 900, "current": 700, "block_size": 8, "page_size": 65536 }
  ],
  "page_bins": [
    { "total": 9, "peak": 9, "current": 0, "block_size": 8, "page_size": 65536 },
    { "total": 9, "peak": 9, "current": 3, "block_size": 112, "page_size": 65536 },
    { "total": 1, "peak": 1, "current": 2, "block_size": 524296, "page_size": 0 }
  ],
  "chunk_bins": [
    { "total": 5, "peak": 5, "current": 5, "bin": "S" }
  ]
}"#;
        assert_eq!(in_use(json), Some(1000 + 3 * 65536));
        let behind = json.replace("\"current\": 1000", "\"current\": -400000");
        assert_eq!(in_use(&behind), Some(0));
    }

    use libmimalloc_sys::{mi_free, mi_heap_delete, mi_heap_malloc, mi_heap_new, mi_heap_t};

    unsafe extern "C" {
        fn mi_heap_stats_get_json(
            heap: *mut mi_heap_t,
            buf_size: usize,
            buf: *mut std::ffi::c_char,
        ) -> *mut std::ffi::c_char;
    }

    /// The bytes in use in `heap` alone, which the other tests' threads never allocate from.
    fn heap_live_bytes(heap: *mut mi_heap_t) -> usize {
        // SAFETY: `heap` is live; a null buffer asks mimalloc for one, which `take_json` frees.
        let raw = unsafe { mi_heap_stats_get_json(heap, 0, std::ptr::null_mut()) };
        in_use(&take_json(raw).unwrap()).unwrap()
    }

    #[test]
    fn live_bytes_follow_large_blocks_in_and_out() {
        // SAFETY: a heap of this test's own, deleted at the end.
        let heap = unsafe { mi_heap_new() };
        let before = heap_live_bytes(heap);
        // SAFETY: allocates from `heap` and frees what it allocated.
        let during = unsafe {
            let block = mi_heap_malloc(heap, 64 << 20);
            let during = heap_live_bytes(heap);
            mi_free(block);
            for _ in 0..8 {
                mi_free(mi_heap_malloc(heap, 64 << 20));
            }
            during
        };
        let after = heap_live_bytes(heap);
        // SAFETY: nothing allocated from `heap` is still live.
        unsafe { mi_heap_delete(heap) };
        assert!(during >= before + (64 << 20), "{before} then {during}");
        assert_eq!(
            after, before,
            "{before} then {after} after freeing nine huge blocks"
        );
    }
}
