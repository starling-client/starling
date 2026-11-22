extern crate sunrise_client_lib as sunrise_client;
use sunrise_core::vanilla_bytes::Writer;
use sunrise_protocol::{
    buf::{Codec, Reader},
    item::Slot,
};

const FIRST_CUSTOM_ID: i32 = 122;

#[test]
fn unregistered_custom_component_id_is_unknown() {
    let mut bytes = Writer::new();
    bytes.varint(1);
    bytes.varint(0);
    bytes.varint(1);
    bytes.varint(0);
    bytes.varint(FIRST_CUSTOM_ID);
    let error = Slot::read(&mut Reader::new(&bytes.data)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains(&format!("unsupported item component {FIRST_CUSTOM_ID}"))
    );
}
