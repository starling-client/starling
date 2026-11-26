//! Starling's tests of java.rs, out of Sunrise's file (tools/starling/move_out.py).
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

#[test]
fn small_android_caps_large_heaps_without_enlarging_small_requests() {
    for heap in ["2G", "1G", "768M"] {
        assert_eq!(heap_for(heap, Some(512)), "-Xmx512M");
        assert_eq!(heap_for(heap, Some(384)), "-Xmx384M");
        assert_eq!(heap_for(heap, None), format!("-Xmx{heap}"));
    }
    assert_eq!(heap_for("512M", Some(384)), "-Xmx384M");
    assert_eq!(heap_for("128M", Some(384)), "-Xmx128M");
}

#[test]
fn small_phones_reserve_more_memory_for_android_and_the_renderer() {
    assert_eq!(
        small_heap_limit("MemTotal: 913568 kB\nMemFree: 12345 kB\n"),
        384
    );
    assert_eq!(small_heap_limit("MemTotal: 1572863 kB\n"), 384);
    assert_eq!(small_heap_limit("MemTotal: 1572864 kB\n"), 512);
    assert_eq!(small_heap_limit("MemTotal: 8388608 kB\n"), 512);
    assert_eq!(small_heap_limit("unavailable"), 384);
    assert_eq!(small_heap_limit("MemTotal: invalid kB"), 384);
}
