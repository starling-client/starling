//! Starling's tests of crafting.rs.
use super::*;

/// A recipe file whose serializer no mod registered is left out and the others load, while
/// leniency is on (`LENIENT_RECIPE_FILES`, the game's start): a content mod without its code half
/// loses those recipes, not the world. Off, the same file refuses the load.
#[test]
fn an_unregistered_serializer_leaves_its_file_out_when_lenient() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("data/minecraft/recipe");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("valid.json"), r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:stick","count":3}}"#).unwrap();
    std::fs::write(
        folder.join("unknown.json"),
        r#"{"type":"absent:serializer"}"#,
    )
    .unwrap();
    let items = Items::for_tests(Default::default());
    let load = crate::test_resources::server_load();
    // On this thread only: the strict tests beside it keep the strict load.
    super::LENIENT_ON_THIS_THREAD.with(|on| on.set(true));
    let lenient = Recipes::load(root.path(), &items, &PackOverlay::NONE, &load);
    super::LENIENT_ON_THIS_THREAD.with(|on| on.set(false));
    assert!(
        lenient.is_ok(),
        "{:?}",
        lenient.err().map(|e| e.to_string())
    );
    let strict = Recipes::load(root.path(), &items, &PackOverlay::NONE, &load);
    assert!(strict.is_err());
}
