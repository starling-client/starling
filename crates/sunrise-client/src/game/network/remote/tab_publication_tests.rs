use crate::game::remote::{object_skin_changed, skin_entries};

/// An NPC plugin lists the NPC, spawns it, and takes the entry off the list:
/// the entity keeps the skin it found; a player spawned before its entry
/// arrives takes the entry once it comes.
#[test]
fn a_player_entity_keeps_the_skin_its_entry_gave() {
    use std::collections::HashMap;
    use sunrise_client::skins::Textures;
    let npc = Textures {
        skin: Some("http://textures.minecraft.net/texture/npc".into()),
        signed: true,
        ..Textures::default()
    };
    let (mut listed, mut kept) = (HashMap::from([(7u128, npc.clone())]), HashMap::new());
    let mut players = HashMap::from([(2i32, 7u128), (3, 8)]);
    assert_eq!(
        skin_entries(&players, &listed, &mut kept),
        [(7, npc.clone())]
    );
    listed.remove(&7);
    assert_eq!(
        skin_entries(&players, &listed, &mut kept),
        [(7, npc.clone())]
    );
    // Player 8's entry comes after its entity.
    let late = Textures {
        skin: Some("http://textures.minecraft.net/texture/late".into()),
        ..npc.clone()
    };
    listed.insert(8, late.clone());
    let mut entries = skin_entries(&players, &listed, &mut kept);
    entries.sort_by_key(|(u, _)| *u);
    assert_eq!(entries, [(7, npc), (8, late)]);
    // A removed entity forgets its profile with it.
    players.remove(&2);
    kept.remove(&2);
    listed.clear();
    assert_eq!(skin_entries(&players, &listed, &mut kept).len(), 1);
}

#[test]
fn tab_publication_fingerprint_tracks_all_inputs() {
    let fingerprint = |ids: &[u128], skins, atlas| {
        (
            ids.len(),
            ids.iter().fold(0, |xor, uuid| xor ^ uuid),
            skins,
            atlas,
        )
    };
    let published = fingerprint(&[7, 8], 1, 2);
    assert_eq!(published, fingerprint(&[8, 7], 1, 2));
    assert_ne!(published, fingerprint(&[7, 9], 1, 2));
    assert_ne!(published, fingerprint(&[7, 8], 2, 2));
    assert_ne!(published, fingerprint(&[7, 8], 1, 3));
}

#[test]
fn tab_object_skin_publication_tracks_changes_and_atlas_clear() {
    let mut published = std::collections::HashMap::new();
    let mut writes = 0;
    for skin in ["skins/first", "skins/first", "skins/second"] {
        for key in ["id:7", "name:player"] {
            if object_skin_changed(&published, key, skin) {
                published.insert(key.to_owned(), skin.to_owned());
                writes += 1;
            }
        }
    }
    assert_eq!(writes, 4, "unchanged skins do not publish either entry");
    published.clear();
    assert!(object_skin_changed(&published, "id:7", "skins/second"));
    assert!(object_skin_changed(
        &published,
        "name:player",
        "skins/second"
    ));
}
