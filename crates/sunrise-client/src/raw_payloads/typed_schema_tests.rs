//! A typed declaration's schema on the negotiated route: the real registrar's manifest against a
//! peer whose value type has another shape.
use super::{Local, Side};
use mod_api::{Hooks, IPayloadContext, ModEntry, PayloadId, PayloadSendError, Registrar, Wire};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use sunrise_core::nbt::{Compound, JavaString, Tag};
use sunrise_protocol::payload_negotiation::{
    self as negotiation, Control, Flow, Manifest, Mismatch, Negotiation, Phase, RegistrationPhase,
};

#[test]
fn required_collection_schemas_with_different_budgets_do_not_negotiate() {
    fn required<T: mod_api::wire::Wire + 'static>() -> Manifest {
        let mut builder = Manifest::builder();
        builder
            .register(
                RegistrationPhase::Play,
                "typed_schema:collection",
                Flow::Bidirectional,
                &mod_api::typed_payload_version("1", mod_api::payload_schema::<T>()),
                false,
            )
            .unwrap();
        builder.finish().unwrap()
    }

    // The entry encodings match, but the decoders charge for different backing
    // storage. A required channel must refuse a peer using the other decoder.
    for (local, peer) in [
        (
            required::<BTreeMap<u8, u8>>(),
            required::<HashMap<u8, u8>>(),
        ),
        (required::<BTreeSet<u8>>(), required::<HashSet<u8>>()),
        (
            required::<Compound>(),
            required::<BTreeMap<JavaString, Tag>>(),
        ),
    ] {
        let refused = Negotiation::new(local).receive(Control::Offer(peer));
        assert!(matches!(refused, Err(negotiation::Error::Incompatible {
            ref id, reason: Mismatch::Schema, ..
        }) if id.as_str() == "typed_schema:collection"));
    }
}

#[derive(Wire)]
struct Counter {
    counter: i32,
}
// The former `struct Counter { counter: VarInt }` payload fingerprint.
const OLD_SCHEMA: u64 = 0xf0f2_25e6_8e40_be70;

struct Typed;
#[mod_api::__subscriptions]
impl Hooks for Typed {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let ignore = |_: Counter, _: &mut dyn IPayloadContext| Ok(());
        let mut payloads = registrar.payload_handlers();
        payloads.registrar("1").play_bidirectional_typed(
            PayloadId("typed_schema:required".into()),
            ignore,
            ignore,
        );
        payloads.registrar("1").optional().play_bidirectional_typed(
            PayloadId("typed_schema:optional".into()),
            ignore,
            ignore,
        );
    }
}

fn peer(optional_too: bool) -> Manifest {
    let old = mod_api::typed_payload_version("1", OLD_SCHEMA);
    let current = mod_api::typed_payload_version("1", mod_api::payload_schema::<Counter>());
    let mut builder = Manifest::builder();
    let required = if optional_too { &current } else { &old };
    builder
        .register(
            RegistrationPhase::Play,
            "typed_schema:required",
            Flow::Bidirectional,
            required,
            false,
        )
        .unwrap();
    builder
        .register(
            RegistrationPhase::Play,
            "typed_schema:optional",
            Flow::Bidirectional,
            &old,
            true,
        )
        .unwrap();
    builder.finish().unwrap()
}

#[test]
fn another_value_schema_is_refused_when_required_and_unsupported_when_optional() {
    assert_eq!(negotiation::SCHEMA_MARKER, mod_api::PAYLOAD_SCHEMA_MARKER);
    mod_platform::testing::with_mods(
        vec![ModEntry::new("typed_schema", Box::new(Typed))].into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            for side in [Side::Client, Side::Server] {
                let local = Local::capture(side).unwrap();
                // Required schema 2 against the peer's schema 1: admission names both versions.
                let refused = Negotiation::new(local.manifest.clone())
                    .receive(Control::Offer(peer(false)))
                    .unwrap_err();
                let negotiation::Error::Incompatible {
                    id,
                    reason,
                    local: ours,
                    peer: theirs,
                    ..
                } = refused
                else {
                    panic!("{refused}");
                };
                assert_eq!(
                    (id.as_str(), reason),
                    ("typed_schema:required", Mismatch::Schema)
                );
                assert_ne!(ours.unwrap().0, theirs.unwrap().0);
                // The optional one with another schema is absent: a send is UnsupportedPeer.
                let mut admitted = Negotiation::new(local.manifest.clone());
                admitted.receive(Control::Offer(peer(true))).unwrap();
                let accepted = admitted.accepted().unwrap();
                let ids: Vec<_> = accepted.iter().map(|entry| entry.id()).collect();
                assert_eq!(ids, ["typed_schema:required"]);
                let table = local.freeze(accepted, side).unwrap();
                let optional = PayloadId("typed_schema:optional".into());
                assert_eq!(
                    table.can_send(Phase::Play, &optional),
                    Err(PayloadSendError::UnsupportedPeer(optional.clone()))
                );
            }
        },
    );
}

struct OwnedText;

#[mod_api::__subscriptions]
impl Hooks for OwnedText {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let unreachable =
            |_: String, _: &mut dyn IPayloadContext| panic!("rejected channel dispatched");
        registrar
            .payload_handlers()
            .registrar("1")
            .play_bidirectional_typed(
                PayloadId("typed_schema:owned".into()),
                unreachable,
                unreachable,
            );
    }
}

#[test]
fn checked_model_json_cannot_negotiate_as_owned_text() {
    mod_platform::testing::with_mods(
        vec![ModEntry::new("typed_schema", Box::new(OwnedText))].into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            let model = mod_api::payload_schema::<mod_api::ModelPart>();
            for side in [Side::Client, Side::Server] {
                let local = Local::capture(side).unwrap();
                let mut peer = Manifest::builder();
                peer.register(
                    RegistrationPhase::Play,
                    "typed_schema:owned",
                    Flow::Bidirectional,
                    &mod_api::typed_payload_version("1", model),
                    false,
                )
                .unwrap();
                let refused = Negotiation::new(local.manifest.clone())
                    .receive(Control::Offer(peer.finish().unwrap()));
                assert!(matches!(refused, Err(negotiation::Error::Incompatible {
                    ref id, reason: Mismatch::Schema, ..
                }) if id.as_str() == "typed_schema:owned"));
            }
        },
    );
}
