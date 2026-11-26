//! `Frog.DATA_VARIANT_ID` (entry 18) carries the synchronized `frog_variant`
//! registry's network id: what the host sends is what the client stores.
use super::super::lan;
use super::apply_registry_data;
use super::pet_pose_meta_tests::meta_hub_with;
use super::tests::fixture_mob;
use sunrise_client::lan::data::{RegistryRef, SyncData};
use sunrise_protocol::entity_data::{Value, serializer};
use sunrise_protocol::packets::configuration::{RegistryData, RegistryEntry};

fn sent(
    m: &sunrise_client::mobs::Mob,
    hub: &sunrise_client::lan::hub::Hub,
) -> Vec<(u8, i32, Value)> {
    let (mut meta, mut defaults) = (Vec::new(), Vec::new());
    lan::mob_meta(m, hub, None, 0, &mut meta, &mut defaults);
    let default = defaults.iter().find(|(index, _, _)| *index == 18);
    assert_eq!(
        default,
        Some(&(18, serializer::FROG_VARIANT, Value::Id(0))),
        "the default is minecraft:temperate's id"
    );
    meta.into_iter()
        .filter(|(index, _, _)| *index == 18)
        .collect()
}

#[test]
fn a_pack_frog_variant_reaches_the_client_by_network_id() {
    let mut sync = SyncData::empty();
    sync.dynamic.add(RegistryData {
        registry: "minecraft:frog_variant".to_owned(),
        entries: ["minecraft:temperate", "test:glow"]
            .into_iter()
            .map(|id| RegistryEntry {
                id: id.to_owned(),
                data: None,
            })
            .collect(),
    });
    let hub = meta_hub_with(sync);

    let mut host = fixture_mob("minecraft:frog");
    host.extra.frog_variant = Some(RegistryRef::new(1));
    let meta = sent(&host, &hub);
    assert_eq!(meta, vec![(18, serializer::FROG_VARIANT, Value::Id(1))]);

    let mut client = fixture_mob("minecraft:frog");
    apply_registry_data(&mut client, &meta, 0);
    assert_eq!(client.extra.frog_variant, Some(RegistryRef::new(1)));

    // The default variant is not sent: the client keeps its own default.
    host.extra.frog_variant = Some(RegistryRef::new(0));
    assert!(sent(&host, &hub).is_empty());
}
