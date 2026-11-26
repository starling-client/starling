use super::*;
use crate::wire::{Reader, Wire, Writer};
use std::collections::HashSet;

#[test]
fn biome_key_parses_hashes_and_roundtrips_as_identifier() {
    let plains = BiomeKey::parse("minecraft:plains").unwrap();
    let bare = BiomeKey::parse("plains").unwrap();
    assert_eq!(plains, bare);
    assert_eq!(plains.namespace(), "minecraft");
    assert_eq!(plains.path(), "plains");
    assert_eq!(plains.to_string(), "minecraft:plains");
    assert_eq!(
        BiomeKey::parse("Bad:Name"),
        Err(BiomeKeyError::InvalidIdentifier)
    );
    assert_eq!(
        BiomeKey::from_parts("..", "plains"),
        Err(BiomeKeyError::InvalidIdentifier)
    );

    let custom = BiomeKey::from_parts("example", "ash_waste").unwrap();
    let mut set = HashSet::new();
    set.insert(plains.clone());
    set.insert(custom.clone());
    assert!(set.contains(&bare));
    assert_eq!(set.len(), 2);

    let mut bytes = Writer::new();
    custom.put(&mut bytes);
    let encoded = bytes.into_bytes();
    let decoded = BiomeKey::take(&mut Reader::new(&encoded)).unwrap();
    assert_eq!(decoded, custom);

    // Mod Wire: two little-endian u32 lengths and UTF-8 fields, not the vanilla network codec.
    assert_eq!(encoded.as_slice(), b"\x07\0\0\0example\x09\0\0\0ash_waste");
}

#[test]
fn biome_views_are_bound_to_their_installed_generation_while_keys_persist() {
    let plains = BiomeKey::parse("minecraft:plains").unwrap();
    let retired = BiomeKey::parse("example:old_only").unwrap();
    let old = BiomeKeys::new(1, [plains.clone(), retired]);
    let new = BiomeKeys::new(2, [plains]);

    let old_plains = old.view("minecraft:plains").unwrap();
    let new_plains = new.view("minecraft:plains").unwrap();
    assert_eq!(old_plains.id(), "minecraft:plains");
    assert_eq!(old_plains.key(), new_plains.key());
    assert!(old_plains.is(&BiomeKey::parse("minecraft:plains").unwrap()));
    assert_ne!(
        old_plains, new_plains,
        "views from distinct generations are never equal"
    );

    assert_eq!(old.id(old_plains), Ok("minecraft:plains"));
    assert_eq!(new.id(old_plains), Err(BiomeKeyError::WrongGeneration));
    assert_eq!(old.view("test:nope"), Err(BiomeKeyError::UnknownBiome));
    assert_eq!(old.view("Bad Name"), Err(BiomeKeyError::InvalidIdentifier));

    // A persistent BiomeKey survives Wire roundtrip and re-resolves against the new generation.
    let kept = old_plains.key().clone();
    let mut bytes = Writer::new();
    kept.put(&mut bytes);
    let decoded = BiomeKey::take(&mut Reader::new(&bytes.into_bytes())).unwrap();
    assert_eq!(new.view_key(&decoded), Ok(new_plains));

    let removed = old.view("example:old_only").unwrap().key().clone();
    assert_eq!(new.view_key(&removed), Err(BiomeKeyError::UnknownBiome));
}
