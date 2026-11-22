use mod_api::wire::{WireError, Writer};

#[test]
fn byte_limits_refuse_before_allocating_and_reset_with_the_buffer() {
    let mut writer = Writer::new();
    writer.limit(8);
    writer.put_bytes(&[0; 1024]);
    assert_eq!(writer.error(), Some(WireError::Length));
    assert!(writer.bytes().is_empty());
    assert_eq!(writer.into_bytes().capacity(), 0);

    let mut writer = Writer::new();
    writer.limit(8);
    writer.put_str("12345"); // Nine bytes including the length.
    assert_eq!(writer.error(), Some(WireError::Length));
    assert!(writer.bytes().is_empty());
    writer.fail(WireError::Utf8);
    assert_eq!(
        writer.error(),
        Some(WireError::Length),
        "the first error wins"
    );
    writer.clear();
    writer.put_str("12345");
    assert_eq!(writer.error(), None);
    assert_eq!(writer.bytes().len(), 9);
    writer.limit(0);
    assert_eq!(writer.error(), Some(WireError::Length));
    writer.replace(Vec::new());
    writer.put(&9u32);
    assert_eq!(writer.error(), None);
    assert_eq!(writer.bytes(), 9u32.to_le_bytes());
}
