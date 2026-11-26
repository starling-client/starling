//! `#[derive(Wire)]` works without the implicit prelude and with shadowed standard names.
//! Generated names resolve from the crate roots, while field and `via` types use the module's
//! own scope. The module shadows macros too, since `no_implicit_prelude` leaves them reachable.
use mod_api::wire::{Reader, Wire, WireError, WireSchema, Writer};

#[no_implicit_prelude]
#[allow(non_camel_case_types)]
mod bare {
    type usize = ::core::primitive::u8;
    type u32 = ::core::primitive::u8;
    type u64 = ::core::primitive::u16;

    // Never expanded: a bare `vec!`, `stringify!` or `unreachable!` in derived code would hit these.
    #[allow(unused_macros)]
    macro_rules! vec {
        ($($any:tt)*) => {
            ::core::compile_error!("derived code used the bare vec!")
        };
    }
    #[allow(unused_macros)]
    macro_rules! stringify {
        ($($any:tt)*) => {
            ::core::compile_error!("derived code used the bare stringify!")
        };
    }
    #[allow(unused_macros)]
    macro_rules! unreachable {
        ($($any:tt)*) => {
            ::core::compile_error!("derived code used the bare unreachable!")
        };
    }

    #[derive(Debug, PartialEq, ::mod_api::Wire)]
    pub struct Record {
        pub count: ::core::primitive::u32,
        pub names: ::std::vec::Vec<::std::string::String>,
    }

    #[derive(Debug, PartialEq, ::mod_api::Wire)]
    pub enum Shape {
        Empty,
        Sized(::core::primitive::u32),
        Named { record: Record },
    }

    #[derive(Debug, PartialEq, ::mod_api::Wire)]
    pub struct ShadowRecord {
        pub small: usize,
        pub wide: u64,
    }

    #[derive(Debug, PartialEq, ::mod_api::Wire)]
    pub enum ShadowChoice {
        Empty,
        Full(ShadowRecord),
    }

    #[derive(Debug, PartialEq, ::mod_api::Wire)]
    #[wire(via = u32)]
    pub struct Checked(u32);

    impl ::core::convert::From<&Checked> for u32 {
        fn from(value: &Checked) -> Self {
            value.0
        }
    }

    impl ::core::convert::TryFrom<u32> for Checked {
        type Error = ();

        fn try_from(value: u32) -> ::core::result::Result<Self, Self::Error> {
            if value == 7 {
                ::core::result::Result::Ok(Self(value))
            } else {
                ::core::result::Result::Err(())
            }
        }
    }
}

#[test]
fn derive_round_trips_without_the_implicit_prelude() {
    use bare::{Record, Shape};
    for value in [
        Shape::Empty,
        Shape::Sized(7),
        Shape::Named {
            record: Record {
                count: 2,
                names: vec!["a".into(), "b".into()],
            },
        },
    ] {
        let mut writer = Writer::new();
        writer.put(&value);
        assert_eq!(Reader::decode::<Shape>(writer.bytes()).unwrap(), value);
    }
    let mut unknown = Writer::new();
    unknown.put(&9u32);
    assert!(Reader::decode::<Shape>(unknown.bytes()).is_err());
}

#[test]
fn derive_resolves_shadowed_names() {
    use bare::{Checked, ShadowChoice, ShadowRecord};

    assert_eq!(ShadowRecord::MIN, 3);
    assert_eq!(ShadowChoice::MIN, 4);
    assert_eq!(Checked::MIN, 1);

    // FNV-1a fingerprints of the record, enum and u8 representation formats.
    assert_eq!(
        ShadowRecord::describe(&mut WireSchema::default()),
        0x6cce_06b7_9931_4886,
    );
    assert_eq!(
        ShadowChoice::describe(&mut WireSchema::default()),
        0xe242_31b4_f901_5658,
    );
    assert_eq!(
        Checked::describe(&mut WireSchema::default()),
        0x08c4_8207_b567_53d8,
    );

    let record = ShadowRecord {
        small: 0xa5,
        wide: 0x1234,
    };
    let record_bytes = [0xa5, 0x34, 0x12];
    let mut writer = Writer::new();
    writer.put(&record);
    assert_eq!(writer.bytes(), &record_bytes);
    assert_eq!(
        Reader::decode::<ShadowRecord>(&record_bytes).unwrap(),
        record
    );

    for (choice, bytes) in [
        (ShadowChoice::Empty, [0, 0, 0, 0].as_slice()),
        (ShadowChoice::Full(record), &[1, 0, 0, 0, 0xa5, 0x34, 0x12]),
    ] {
        writer.clear();
        writer.put(&choice);
        assert_eq!(writer.bytes(), bytes);
        assert_eq!(Reader::decode::<ShadowChoice>(bytes), Ok(choice));
    }

    let checked = Checked::try_from(7).unwrap();
    let checked_bytes = [7];
    writer.clear();
    writer.put(&checked);
    assert_eq!(writer.bytes(), &checked_bytes);
    assert_eq!(Reader::decode::<Checked>(&checked_bytes), Ok(checked));

    assert_eq!(
        Reader::decode::<ShadowChoice>(&[9, 0, 0, 0]),
        Err(WireError::Tag(9))
    );
    assert_eq!(
        Reader::decode::<ShadowChoice>(&[1, 0, 0]),
        Err(WireError::Eof)
    );
    assert_eq!(
        Reader::decode::<Checked>(&[8]),
        Err(WireError::Invalid("Checked"))
    );
}
