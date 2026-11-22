//! The attachment and its save/load edits used by the region writer tests.
use mod_api::{
    AttachmentCodec, AttachmentError, AttachmentType, ChunkDiskData, ChunkDiskHost, Compound, Tag,
};

pub const VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "chunk_disk_fixture",
    name: "disk_value",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |value| Ok(Compound::from_iter([("counter".into(), Tag::Int(*value))])),
        decode: |tag| match tag.get(&"counter".into()) {
            Some(Tag::Int(value)) => Ok(*value),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

/// Only the chunk that the test fills with a value needs editing.
pub fn save(data: &mut ChunkDiskData) {
    let value = data.get(&VALUE).unwrap();
    if value == 0 {
        return;
    }
    assert_eq!(value, 7);
    data.set(&VALUE, 8).unwrap();
}

pub fn load(_: &dyn ChunkDiskHost, data: &mut ChunkDiskData) {
    let value = data.get(&VALUE).unwrap();
    if value != 0 {
        data.set(&VALUE, value + 1).unwrap();
    }
}
