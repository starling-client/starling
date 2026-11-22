//! `#[derive(Wire)]` in a module without the implicit prelude (#1510): the derived code names
//! the standard types, constructors and macros from the crate roots, so it compiles and
//! round-trips where none of the prelude's names are in scope. `no_implicit_prelude` leaves
//! the standard macros reachable, so the module shadows the ones the derive could use.
use mod_api::wire::{Reader, Writer};

#[no_implicit_prelude]
mod bare {
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
        pub count: u32,
        pub names: ::std::vec::Vec<::std::string::String>,
    }

    #[derive(Debug, PartialEq, ::mod_api::Wire)]
    pub enum Shape {
        Empty,
        Sized(u32),
        Named { record: Record },
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
