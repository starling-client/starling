use sunrise_mod_package::{Capability, Manifest, manifest::Side};

#[test]
fn published_native_manifests_validate_unchanged() {
    for (id, text, side, engine_patches) in [
        (
            "supercapes",
            include_str!("fixtures/registry-manifests/supercapes.toml"),
            Side::Client,
            true,
        ),
        (
            "better_inventory",
            include_str!("fixtures/registry-manifests/better_inventory.toml"),
            Side::Client,
            true,
        ),
        (
            "motion_blur",
            include_str!("fixtures/registry-manifests/motion_blur.toml"),
            Side::Client,
            true,
        ),
        (
            "fireflies",
            include_str!("fixtures/registry-manifests/fireflies.toml"),
            Side::Both,
            false,
        ),
    ] {
        let manifest = Manifest::parse(text).unwrap_or_else(|error| panic!("{id}: {error}"));
        assert_eq!(manifest.id, id);
        assert_eq!(manifest.side, side);
        assert_eq!(manifest.api, "0.2");
        assert_eq!(manifest.capabilities, [Capability::Registries]);
        let native = manifest.native.unwrap();
        assert_eq!(native.game, "26.3");
        assert!(native.mixins.is_some(), "{id}");
        assert_eq!(!native.mixin.is_empty(), engine_patches, "{id}");
    }
}
