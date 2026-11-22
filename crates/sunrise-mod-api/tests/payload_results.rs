//! Public payload callback results use the canonical Wire codec.
use mod_api::PayloadHandleError;
use mod_api::wire::{Reader, WireError, Writer};

#[test]
fn callback_success_and_real_failure_have_distinct_wire_results() {
    let mut out = Writer::new();
    out.put(&Ok::<(), PayloadHandleError>(()));
    assert_eq!(out.error(), None);
    assert_eq!(out.bytes(), &[0]);
    assert_eq!(
        Reader::decode::<Result<(), PayloadHandleError>>(out.bytes()),
        Ok(Ok(()))
    );

    out.clear();
    let result = Err::<(), _>(PayloadHandleError::Failed("bad".into()));
    out.put(&result);
    assert_eq!(out.error(), None);
    assert_eq!(out.bytes(), &[1, 0, 0, 0, 0, 3, 0, 0, 0, b'b', b'a', b'd']);
    assert_eq!(
        Reader::decode::<Result<(), PayloadHandleError>>(out.bytes()),
        Ok(result)
    );
}

#[test]
fn invalid_callback_results_refuse_tags_truncation_and_trailing_data() {
    assert_eq!(
        Reader::decode::<Result<(), PayloadHandleError>>(&[2]),
        Err(WireError::Tag(2))
    );
    assert_eq!(
        Reader::decode::<Result<(), PayloadHandleError>>(&[1, 1, 0, 0, 0]),
        Err(WireError::Tag(1))
    );
    let mut out = Writer::new();
    out.put(&Err::<(), _>(PayloadHandleError::Failed("bad".into())));
    assert_eq!(out.error(), None);
    for end in 0..out.bytes().len() {
        assert!(
            Reader::decode::<Result<(), PayloadHandleError>>(&out.bytes()[..end]).is_err(),
            "accepted a truncated result at {end}"
        );
    }
    let mut trailing = out.bytes().to_vec();
    trailing.push(0);
    assert_eq!(
        Reader::decode::<Result<(), PayloadHandleError>>(&trailing),
        Err(WireError::Trailing)
    );
}
