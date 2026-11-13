//! Execute the game load expression against a real engine registry and count lookup allocations.
use horizon_world::registry::BlockRegistry;
use mod_api::__private::{block_key, block_key_id};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! { static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) }; }
struct Allocator;
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
fn allocation() {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n + 1));
        }
    });
}
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        allocation();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

fn complete(registry: &BlockRegistry) -> Result<(), String> {
    // PRODUCTION_COMPLETION
    Ok(())
}

#[test]
fn production_block_key_load_boundary() {
    mod_api::__private::part_tests::install_parts_fixture(&[]).unwrap();
    let facts = br#"{"defaults":{"minecraft:air":1,"minecraft:stone":2,"minecraft:zombie_head":0,"probe:machine":3},"states":[
        ["z",false,false,"","minecraft:zombie_head"],
        ["a",true,false,"","minecraft:air"],
        ["s",false,false,"","minecraft:stone"],
        ["p",false,false,"",{"id":"probe:machine","properties":{"lit":"false"}}],
        ["p",false,false,"",{"id":"probe:machine","properties":{"lit":"true"}}]
    ]}"#;
    let registry = BlockRegistry::from_facts_json(&facts[..]).unwrap();
    complete(&registry).unwrap();
    let names = [
        "minecraft:air",
        "minecraft:stone",
        "minecraft:zombie_head",
        "probe:machine",
    ];
    for name in names {
        let key = block_key(name).unwrap();
        assert_eq!(block_key_id(key), Some(name));
        assert_eq!(
            registry.default_state(block_key_id(key).unwrap()),
            registry.default_state(name)
        );
    }
    let air = block_key(names[0]).unwrap();
    ALLOCATIONS.set(Some(0));
    for _ in 0..1_000 {
        assert_eq!(block_key("minecraft:air"), Some(air));
        assert_eq!(block_key("absent:block"), None);
    }
    let allocations = ALLOCATIONS.replace(None).unwrap();
    assert_eq!(
        allocations, 0,
        "published key lookups must borrow the completed table"
    );
}
