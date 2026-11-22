use crate::{DataMap, DataMapError, GameEventKey, GameEventKeyError, Host, RegistryName, Tag};
struct Frequencies;
impl DataMap for Frequencies {
    type Entry = GameEventKey;
    type Value = i32;
    fn id(&self) -> &str {
        "neoforge:vibration_frequencies"
    }
    fn decode(&self, value: &Tag) -> Option<i32> {
        if let Tag::Int(value) = value {
            Some(*value)
        } else {
            None
        }
    }
}
struct Maps;
impl crate::host::keys::KeysBackend for Maps {
    fn game_event_key_id(&self, key: GameEventKey) -> Result<RegistryName, GameEventKeyError> {
        match key.0 {
            0 => Ok(RegistryName::from_static("minecraft:step")),
            1 => Ok(RegistryName::from_static("minecraft:swim")),
            _ => Err(GameEventKeyError::InvalidKey),
        }
    }
}
impl crate::host::data_maps::DataMapsBackend for Maps {
    fn data_map_value(
        &self,
        registry: &str,
        entry: &str,
        map: &str,
    ) -> Result<Option<&Tag>, DataMapError> {
        static STEP_FREQUENCY: Tag = Tag::Int(7);
        assert_eq!(registry, "minecraft:game_event");
        assert_eq!(map, "neoforge:vibration_frequencies");
        Ok((entry == "minecraft:step").then_some(&STEP_FREQUENCY))
    }
}
impl Host for Maps {
    fn backend(&self) -> Option<crate::BackendRef<'_>> {
        Some(crate::BackendRef::of(self))
    }
}
impl crate::Backend for Maps {}
crate::impl_host_areas!(Maps);
crate::impl_backend_areas!(Maps, except(keys, data_maps));
struct Bare;
impl Host for Bare {}
crate::impl_host_areas!(Bare);
#[test]
fn registry_names_use_text_identity_across_backing_tables() {
    use crate::content::registry_keys::registry_name_snapshot;
    use std::{cmp::Ordering, collections::HashMap};
    use sunrise_mod_runtime::registry_keys::Names;

    let resident = GameEventKey(0).id(&Maps).unwrap();
    let mut table = Names::default();
    let slot = table.request("fixture", "minecraft:step").unwrap();
    let snapshot = registry_name_snapshot(&table, slot).unwrap();
    drop(table);

    assert_eq!(snapshot, resident);
    assert_eq!(snapshot, "minecraft:step");
    assert_eq!("minecraft:step", snapshot);
    assert!(PartialEq::<str>::eq(&snapshot, "minecraft:step"));
    assert_eq!(AsRef::<str>::as_ref(&snapshot), "minecraft:step");
    assert_eq!(format!("{snapshot}"), "minecraft:step");
    assert!(std::ptr::eq(snapshot.as_str(), snapshot.clone().as_str()));

    let counts = HashMap::from([(resident, 7)]);
    assert_eq!(counts.get(&snapshot), Some(&7));
    assert_eq!(counts.get("minecraft:step"), Some(&7));
    let later = RegistryName::from_static("z:a");
    assert_eq!(snapshot.cmp(&later), Ordering::Less);
    assert!(snapshot < later);
}

#[test]
fn game_event_data_reads_exact_frequency_missing_and_invalid_slots() {
    assert_eq!(GameEventKey(0).id(&Maps).unwrap(), "minecraft:step");
    assert_eq!(GameEventKey(0).data(&Maps, &Frequencies), Ok(Some(7)));
    assert_eq!(GameEventKey(1).data(&Maps, &Frequencies), Ok(None));
    assert_eq!(
        GameEventKey(u32::MAX).id(&Maps),
        Err(GameEventKeyError::InvalidKey)
    );
    assert_eq!(
        GameEventKey(u32::MAX).data(&Maps, &Frequencies),
        Err(DataMapError::InvalidKey)
    );
    assert_eq!(
        GameEventKey(0).data(&Bare, &Frequencies),
        Err(DataMapError::Unavailable)
    );
    assert_eq!(
        GameEventKey(0).id(&Bare),
        Err(GameEventKeyError::Unavailable)
    );
    assert!(matches!(
        crate::wire::Reader::decode::<GameEventKey>(&u32::MAX.to_le_bytes()),
        Err(crate::wire::WireError::Invalid("unknown GameEventKey slot"))
    ));
}
