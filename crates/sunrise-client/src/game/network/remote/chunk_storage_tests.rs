#[test]
fn the_chunk_storage_takes_any_radius_and_centre_a_server_sends() {
    use crate::game::remote::in_storage_range;
    // Vanilla's storage: radius at least 2, plus 3, each way from the centre.
    assert!(in_storage_range(10, (0, 0), 13, -13));
    assert!(!in_storage_range(10, (0, 0), 14, 0));
    assert!(in_storage_range(-5, (100, 100), 105, 95));
    assert!(!in_storage_range(0, (100, 100), 106, 100));
    // Out of i32's range, as a server may send: no overflow, and the sums a wrap got wrong.
    assert!(in_storage_range(i32::MAX, (0, 0), i32::MAX, i32::MIN + 1));
    assert!(!in_storage_range(10, (i32::MIN, 0), i32::MAX, 0));
    assert!(in_storage_range(
        10,
        (i32::MIN, i32::MAX),
        i32::MIN + 13,
        i32::MAX - 13
    ));
}
