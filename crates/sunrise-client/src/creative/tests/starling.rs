//! Starling's tests of creative.rs, out of Sunrise's file (tools/starling/move_out.py).
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

#[test]
fn six_rows_of_items_have_no_gap() {
    for (slot, expected) in [(0, (9.0, 18.0)), (44, (153.0, 90.0)), (45, (9.0, 108.0))] {
        assert_eq!(
            Creative::tab_slot_position(Creative::DISPLAY + slot),
            Some(expected)
        );
    }
}


#[test]
fn the_item_grid_takes_its_cells_the_pixels_between_them_and_a_margin() {
    // A cell's middle, the pixels between two items, and the grid's corners past the cells.
    for mouse in [(17.0, 26.0), (25.5, 35.5), (4.0, 13.0), (173.9, 110.9)] {
        assert!(Creative::on_item_grid(mouse, 5), "{mouse:?}");
    }
    // Past the margin, the scrollbar, and the hotbar under five rows.
    for mouse in [(3.9, 26.0), (17.0, 12.9), (175.0, 40.0), (17.0, 112.0)] {
        assert!(!Creative::on_item_grid(mouse, 5), "{mouse:?}");
    }
    // Bedrock Inventory's sixth row, which has no hotbar under it.
    assert!(Creative::on_item_grid((17.0, 116.0), 6));
    assert!(!Creative::on_item_grid((17.0, 129.0), 6));
}


#[test]
fn a_finger_means_the_destroy_item_slot_a_little_past_it() {
    // Its middle, and 11 pixels off it (half a cell and 2 beyond).
    assert!(Creative::near_trash((181.0, 120.0), 11.0));
    assert!(Creative::near_trash((170.0, 131.0), 11.0));
    // The last hotbar slot's middle, beside it.
    assert!(!Creative::near_trash((161.0, 120.0), 11.0));
    assert!(!Creative::near_trash((181.0, 132.0), 11.0));
}
