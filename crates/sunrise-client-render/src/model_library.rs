//! Parsed resource models for one startup or resource-pack reload.
use crate::{
    Result,
    mod_model_loaders::{ModelLoaderTable, decode_model_with_loaders, installed_loaders},
    resource_source::ResourceSource,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use sunrise_assets::{
    model::CuboidModel,
    resolve::{AssetReader, ModelLibrary},
};

#[derive(Default)]
pub struct ParsedModels {
    models: BTreeMap<String, Arc<CuboidModel>>,
    /// Files whose load failure has already been reported for this generation.
    rejected: BTreeSet<String>,
    /// `None` is an unscanned source, used by callers that only read selected models.
    source: Option<ResourceSource>,
}

fn read_model(
    source: &ResourceSource,
    path: &str,
    loaders: Option<&ModelLoaderTable>,
) -> std::result::Result<Arc<CuboidModel>, String> {
    let text = source
        .read_to_string(path)
        .map_err(|error| error.to_string())?;
    decode_model_with_loaders(&text, loaders)
        .map(Arc::new)
        .map_err(|error| error.to_string())
}
/// Asset files under a content root, for models that name a file (obj).
struct RootAssets(ResourceSource);
impl AssetReader for RootAssets {
    fn read_text(&self, location: &str) -> Option<String> {
        let (namespace, path) = location.split_once(':')?;
        // Only a resource location's characters, and no `.`/`..` step: an
        // .obj's `mtllib` names its .mtl as raw text, so it must not reach a
        // file outside the content root (`../..:x` as a namespace would).
        let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
        if !namespace.bytes().all(valid)
            || matches!(namespace, "" | "." | "..")
            || !path.bytes().all(|c| valid(c) || c == b'/')
            || path.split('/').any(|part| matches!(part, "" | "." | ".."))
        {
            return None;
        }
        self.0
            .read_to_string(&format!("assets/{namespace}/{path}"))
            .ok()
    }
}
impl ParsedModels {
    /// Every `models/**.json` of a namespace directory, by identifier.
    pub fn load(source: &ResourceSource) -> Self {
        Self::load_with_loaders(source, installed_loaders())
    }

    /// Scan this source with exactly the supplied table, including on the parsing workers.
    pub(crate) fn load_with_loaders(
        source: &ResourceSource,
        loaders: Option<&ModelLoaderTable>,
    ) -> Self {
        let files: Vec<_> = source
            .list_under("assets", &["models"])
            .into_iter()
            .filter_map(|name| {
                let (namespace, path) = name.strip_prefix("assets/")?.split_once('/')?;
                let path = path.strip_prefix("models/")?.strip_suffix(".json")?;
                Some((format!("{namespace}:{path}"), name))
            })
            .collect();
        let parsed = crate::par::par_map(&files, |(_, path)| read_model(source, path, loaders));
        let mut models = BTreeMap::new();
        let mut rejected = BTreeSet::new();
        for ((id, _), result) in files.into_iter().zip(parsed) {
            match result {
                Ok(model) => {
                    models.insert(id, model);
                }
                Err(reason) => {
                    log::error!("Sunrise: failed to load model {id}: {reason}");
                    rejected.insert(id);
                }
            }
        }
        Self {
            models,
            rejected,
            source: Some(source.clone()),
        }
    }

    /// Missing blockstate models and their parents use the resolver's missing model.
    pub fn block_library(
        &self,
        source: &ResourceSource,
        pending: Vec<String>,
    ) -> Result<ModelLibrary> {
        self.reachable_library(source, pending, &[])
    }

    /// Standalone registrations must name existing files, but those files share the
    /// normal model discovery rules for absent or rejected parents.
    pub(crate) fn standalone_library(
        &self,
        source: &ResourceSource,
        roots: &[String],
    ) -> Result<ModelLibrary> {
        self.reachable_library(source, roots.to_vec(), roots)
    }

    fn reachable_library(
        &self,
        source: &ResourceSource,
        mut pending: Vec<String>,
        required_roots: &[String],
    ) -> Result<ModelLibrary> {
        let mut models = BTreeMap::new();
        let mut skipped = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if models.contains_key(&name) || skipped.contains(&name) {
                continue;
            }
            let model = match self.models.get(&name) {
                Some(model) => Ok(Arc::clone(model)),
                None if self.source.is_some() => Err("missing from this model load".into()),
                None => name
                    .split_once(':')
                    .ok_or_else(|| "invalid model identifier".to_owned())
                    .and_then(|(namespace, path)| {
                        read_model(
                            source,
                            &format!("assets/{namespace}/models/{path}.json"),
                            installed_loaders(),
                        )
                    }),
            };
            match model {
                Ok(model) => {
                    pending.extend(model.referenced_models());
                    models.insert(name, model);
                }
                Err(reason) => {
                    if required_roots.contains(&name) {
                        return Err(format!("standalone model file {name}: {reason}").into());
                    }
                    if !self.rejected.contains(&name) {
                        log::warn!("Sunrise: missing model {name}: {reason}");
                    }
                    skipped.insert(name);
                }
            }
        }
        Ok(ModelLibrary::from_shared(models)?.with_assets(Arc::new(RootAssets(source.clone()))))
    }

    pub fn into_item_library(self) -> std::result::Result<Arc<ModelLibrary>, String> {
        let source = self.source.unwrap_or_default();
        ModelLibrary::with_shared_item_generator(self.models)
            .map(|library| Arc::new(library.with_assets(Arc::new(RootAssets(source)))))
            .map_err(|e| format!("item models: {e:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use sunrise_assets::resolve::{GENERATED_MODEL, MISSING_MODEL};

    #[test]
    fn archive_models_and_auxiliary_files_load_without_an_asset_directory() -> Result<()> {
        use std::io::Write;
        let fixture = Fixture(
            std::env::temp_dir().join(format!("sunrise-archive-models-{}", std::process::id())),
        );
        fs::create_dir_all(&fixture.0)?;
        let path = fixture.0.join("models.jar");
        let mut writer = zip::ZipWriter::new(fs::File::create(&path)?);
        for (name, bytes) in [
            ("assets/demo/models/block/base.json", br##"{"textures":{"all":"demo:block/stone"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"north":{"texture":"#all"}}}]}"##.as_slice()),
            ("assets/demo/models/block/child.json", br#"{"parent":"demo:block/base"}"#),
            ("assets/demo/models/cube.obj", b"mtllib cube.mtl\nv 0 0 0\n"),
            ("assets/demo/models/cube.mtl", b"newmtl cube\n"),
        ] {
            writer.start_file(name, zip::write::SimpleFileOptions::default())?;
            writer.write_all(bytes)?;
        }
        writer.finish()?;
        let source = ResourceSource::archive(&path)?;
        let parsed = ParsedModels::load(&source);
        let library = parsed.block_library(&source, vec!["demo:block/child".into()])?;
        let child = library.resolve("demo:block/child")?;
        assert_eq!(child.elements().len(), 1);
        assert_eq!(child.material("all")?.unwrap().sprite, "demo:block/stone");
        let assets = RootAssets(source);
        assert_eq!(
            assets.read_text("demo:models/cube.obj").as_deref(),
            Some("mtllib cube.mtl\nv 0 0 0\n")
        );
        assert_eq!(
            assets.read_text("demo:models/cube.mtl").as_deref(),
            Some("newmtl cube\n")
        );
        assert!(!fixture.0.join("assets").exists());
        assert_eq!(fs::read_dir(&fixture.0)?.count(), 1);
        Ok(())
    }

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn write(&self, name: &str, json: &str) {
            let (namespace, path) = name.split_once(':').unwrap();
            let path = self
                .0
                .join(format!("assets/{namespace}/models/{path}.json"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, json).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn an_unknown_required_loader_replaces_only_its_model_in_the_parent_chain() -> Result<()> {
        use sunrise_assets::bake::{BlockModelTransform, bake_block_model};

        let fixture = Fixture(
            std::env::temp_dir().join(format!("sunrise-rejected-model-{}", std::process::id())),
        );
        fixture.write("demo:block/rejected", r#"{"loader":"demo:unregistered"}"#);
        fixture.write(
            "demo:block/child",
            r##"{"parent":"demo:block/rejected","textures":{"side":"demo:block/stone"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#side"}}}]}"##,
        );
        let source = ResourceSource::directory(fixture.0.clone());
        let parsed = ParsedModels::load(&source);
        let blocks = parsed.block_library(
            &source,
            vec!["demo:block/rejected".into(), "demo:block/child".into()],
        )?;
        let items = parsed.into_item_library()?;
        for library in [&blocks, items.as_ref()] {
            let transform = BlockModelTransform::new([0; 3], false);
            let missing = bake_block_model(&library.resolve("demo:block/rejected")?, transform)?;
            assert_eq!(missing.len(), 6);
            assert!(
                missing
                    .iter()
                    .all(|q| q.material.sprite == "minecraft:missingno")
            );
            let child = bake_block_model(&library.resolve("demo:block/child")?, transform)?;
            assert_eq!(child.len(), 1);
            assert_eq!(child[0].material.sprite, "demo:block/stone");
        }
        Ok(())
    }

    #[test]
    fn shared_views_preserve_resolution_errors_builtins_and_reload() -> Result<()> {
        let fixture = Fixture(
            std::env::temp_dir().join(format!("sunrise-shared-models-{}", std::process::id())),
        );
        fixture.write("sunrise:parent", r##"{"textures":{"side":"#all","all":"block/stone"},"elements":[{"from":[0,0,0],"to":[16,8,16],"faces":{"up":{"texture":"#side"}}}]}"##);
        fixture.write("sunrise:nested/child", r#"{"parent":"sunrise:parent","textures":{"all":"block/glass"},"display":{"gui":{"rotation":[30,45,0]}}}"#);
        fixture.write("sunrise:broken", "{");
        fixture.write("sunrise:dangling", r#"{"parent":"sunrise:absent"}"#);
        fixture.write("sunrise:a", r#"{"parent":"sunrise:b"}"#);
        fixture.write("sunrise:b", r#"{"parent":"sunrise:a"}"#);
        fixture.write(
            GENERATED_MODEL,
            r#"{"elements":[{"from":[0,0,0],"to":[1,1,1],"faces":{"up":{"texture":"particle"}}}],"gui_light":"side"}"#,
        );
        fixture.write(MISSING_MODEL, r#"{"elements":[]}"#);
        let parsed = ParsedModels::load(&ResourceSource::directory(fixture.0.clone()));
        let direct = ParsedModels::default();
        let names = vec![
            "sunrise:nested/child".into(),
            GENERATED_MODEL.into(),
            MISSING_MODEL.into(),
            "sunrise:a".into(),
        ];
        let old =
            direct.block_library(&ResourceSource::directory(fixture.0.clone()), names.clone())?;
        let shared = parsed.block_library(&ResourceSource::directory(fixture.0.clone()), names)?;
        for name in ["sunrise:nested/child", GENERATED_MODEL, MISSING_MODEL] {
            assert_eq!(
                format!("{:?}", old.resolve(name)?),
                format!("{:?}", shared.resolve(name)?)
            );
        }
        assert!(shared.resolve("sunrise:a").is_err());
        for name in ["sunrise:broken", "sunrise:absent", "sunrise:dangling"] {
            let source = ResourceSource::directory(fixture.0.clone());
            for models in [&direct, &parsed] {
                let library = models.block_library(&source, vec![name.into()])?;
                assert_eq!(
                    format!("{:?}", library.resolve(name)?.elements()),
                    format!("{:?}", library.resolve(MISSING_MODEL)?.elements()),
                    "{name}"
                );
            }
        }
        let items = parsed.into_item_library()?;
        let block = shared.resolve("sunrise:nested/child")?;
        let item = items.resolve("sunrise:nested/child")?;
        assert!(std::ptr::eq(
            block.elements().as_ptr(),
            item.elements().as_ptr()
        ));
        assert_eq!(
            item.material("side")?.unwrap().sprite,
            "minecraft:block/glass"
        );
        assert_eq!(shared.resolve(GENERATED_MODEL)?.elements().len(), 1);
        assert!(items.resolve(GENERATED_MODEL)?.elements().is_empty());
        assert!(items.resolve(GENERATED_MODEL)?.uses_item_generator());
        for name in [
            "sunrise:broken",
            "sunrise:absent",
            "sunrise:dangling",
            MISSING_MODEL,
        ] {
            assert_eq!(items.resolve(name)?.elements().len(), 1);
        }
        assert!(items.resolve("sunrise:a").is_err());
        fixture.write("sunrise:parent", r#"{"elements":[]}"#);
        let reloaded = ParsedModels::load(&ResourceSource::directory(fixture.0.clone()))
            .into_item_library()?;
        assert!(
            reloaded
                .resolve("sunrise:nested/child")?
                .elements()
                .is_empty()
        );
        assert_eq!(items.resolve("sunrise:nested/child")?.elements().len(), 1);
        // An unrelated bad item identifier must not make block preparation fail
        // (which would trigger resource-pack rollback at a different stage).
        fixture.write("sunrise:INVALID", "{}");
        let parsed = ParsedModels::load(&ResourceSource::directory(fixture.0.clone()));
        assert!(
            parsed
                .block_library(
                    &ResourceSource::directory(fixture.0.clone()),
                    vec!["sunrise:nested/child".into()]
                )
                .is_ok()
        );
        assert!(parsed.into_item_library().is_err());
        Ok(())
    }

    /// An .obj's `mtllib` is raw text: no namespace or path step reaches a
    /// file outside the content root.
    #[test]
    fn root_assets_stay_inside_the_content_root() {
        let root = std::env::temp_dir().join(format!("sunrise-root-assets-{}", std::process::id()));
        let inside = root.join("content");
        fs::create_dir_all(inside.join("assets/m/models")).unwrap();
        fs::write(inside.join("assets/m/models/a.mtl"), "newmtl a").unwrap();
        fs::write(root.join("secret.mtl"), "newmtl secret").unwrap();
        let assets = RootAssets(ResourceSource::directory(inside));
        assert_eq!(
            assets.read_text("m:models/a.mtl").as_deref(),
            Some("newmtl a")
        );
        for escape in [
            "../..:secret.mtl",
            "..:../secret.mtl",
            "m:../../../secret.mtl",
            "m:models/./a.mtl",
            "M:models/a.mtl",
            ":models/a.mtl",
        ] {
            assert_eq!(assets.read_text(escape), None, "{escape}");
        }
        fs::remove_dir_all(&root).unwrap();
    }
}
