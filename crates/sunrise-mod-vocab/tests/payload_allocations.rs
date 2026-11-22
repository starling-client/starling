use mod_vocab::{PAYLOAD_DECODE_BUDGET, decode_payload, encode_payload, wire::WireError};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    hint::black_box,
};

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAllocator;

// SAFETY: all allocation operations forward their arguments unchanged to System.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        unsafe { System.alloc(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        unsafe { System.realloc(ptr, layout, size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Debug, PartialEq, sunrise_mod_api_macros::Wire)]
#[wire(crate = mod_vocab::wire)]
struct Body {
    seq: i32,
    columns: Vec<String>,
}

#[test]
fn typed_text_payloads_do_not_grow_more_than_the_legacy_codec() {
    // Same body shape and sizes as the pilot: each column holds at most 4080 bytes.
    for (text_bytes, legacy_allocations) in [(24usize, 2), (1016, 2), (32640, 5)] {
        let value = Body {
            seq: 1,
            columns: (0..text_bytes.div_ceil(4080))
                .map(|column| "a".repeat((text_bytes - column * 4080).min(4080)))
                .collect(),
        };
        ALLOCATIONS.set(0);
        let result = black_box(encode_payload(black_box(&value), PAYLOAD_DECODE_BUDGET));
        let count = ALLOCATIONS.get();
        let bytes = result.unwrap();
        assert!(
            count <= legacy_allocations,
            "{text_bytes} text bytes: {count} allocations, expected at most {legacy_allocations}"
        );
        assert_eq!(decode_payload(&bytes), Ok(value));
    }
}

#[test]
fn typed_numeric_lists_reserve_once_after_checking_the_bounds() {
    let value = vec![0x0102u16; 255];
    let limit = 4 + 2 * value.len();
    ALLOCATIONS.set(0);
    let result = black_box(encode_payload(black_box(&value), limit));
    let count = ALLOCATIONS.get();
    let bytes = result.unwrap();
    assert_eq!(count, 1);
    assert_eq!(&bytes[..6], &[255, 0, 0, 0, 2, 1]);
    assert_eq!(decode_payload(&bytes), Ok(value.clone()));

    for (value, limit) in [(&value, limit - 1), (&vec![0; 4097], PAYLOAD_DECODE_BUDGET)] {
        ALLOCATIONS.set(0);
        let result = black_box(encode_payload(black_box(value), limit));
        let count = ALLOCATIONS.get();
        assert_eq!(result, Err(WireError::Length));
        assert_eq!(count, 0);
    }
}

#[test]
fn a_five_element_boolean_list_reserves_only_once() {
    let value = vec![true, false, true, false, true];
    ALLOCATIONS.set(0);
    let result = black_box(encode_payload(black_box(&value), 9));
    let count = ALLOCATIONS.get();
    assert_eq!(count, 1);
    assert_eq!(result.unwrap(), [5, 0, 0, 0, 1, 0, 1, 0, 1]);
}

#[test]
fn a_truncated_fixed_array_never_reserves_storage() {
    let bytes = [1u8; 15];
    ALLOCATIONS.set(0);
    let result = black_box(decode_payload::<[u8; 16]>(black_box(&bytes)));
    let count = ALLOCATIONS.get();
    assert_eq!(result, Err(WireError::Eof));
    assert_eq!(count, 0);
}
