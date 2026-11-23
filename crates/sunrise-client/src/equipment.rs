//! Equipment assets (`assets/<namespace>/equipment/<name>.json`): the layer textures a worn item
//! draws, by the asset id its `equippable` names. Vanilla's are `minecraft:diamond` and the like;
//! a server's resource pack adds its own (`nexo:ruby`), which its items name in their own
//! `equippable` component.
use serde_json::Value;
use std::{collections::HashMap, path::Path};

/// Each asset's layers: by kind (`humanoid`, `humanoid_leggings`, `wings`...), the first layer's
/// sprite and, for a dyeable one, its colour when undyed.
pub type Layers = HashMap<String, HashMap<String, (String, Option<u32>)>>;

/// Every equipment asset under `root`, as its id and JSON, vanilla's namespace first.
pub fn assets(root: &Path) -> Vec<(String, Value)> {
    let mut namespaces: Vec<String> = std::fs::read_dir(root.join("assets"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    namespaces.sort_by_key(|n| (n != "minecraft", n.clone()));
    let mut out = Vec::new();
    for ns in namespaces {
        let Ok(entries) = std::fs::read_dir(root.join("assets").join(&ns).join("equipment")) else {
            continue;
        };
        let mut files: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        files.sort();
        for path in files {
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(v) = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            else {
                continue;
            };
            out.push((format!("{ns}:{name}"), v));
        }
    }
    out
}

/// The atlas sprite of layer texture `texture` (`nexo:ruby`) of kind `kind`: vanilla's without its
/// namespace (`entity/equipment/humanoid/diamond`), another namespace's with it
/// (`nexo:entity/equipment/humanoid/ruby`), as the GUI atlas names them.
pub fn sprite(texture: &str, kind: &str) -> String {
    match texture.split_once(':') {
        Some((ns, name)) if ns != "minecraft" => format!("{ns}:entity/equipment/{kind}/{name}"),
        Some((_, name)) => format!("entity/equipment/{kind}/{name}"),
        None => format!("entity/equipment/{kind}/{texture}"),
    }
}

/// Asset `id`'s first layer of each kind.
pub fn layers(id: &str, v: &Value) -> HashMap<String, (String, Option<u32>)> {
    let mut kinds = HashMap::new();
    let Some(all) = v["layers"].as_object() else {
        return kinds;
    };
    for (kind, list) in all {
        if let Some(layer) = list.get(0) {
            let texture = layer["texture"].as_str().unwrap_or(id);
            let undyed = layer["dyeable"]["color_when_undyed"]
                .as_i64()
                .map(|c| c as u32 & 0xFF_FFFF);
            kinds.insert(kind.clone(), (sprite(texture, kind), undyed));
        }
    }
    kinds
}

/// Every asset's layers under `root`.
pub fn load(root: &Path) -> Layers {
    assets(root)
        .into_iter()
        .map(|(id, v)| {
            let kinds = layers(&id, &v);
            (id, kinds)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pack_namespace_keeps_its_namespace_in_the_sprite() {
        assert_eq!(
            sprite("nexo:ruby", "humanoid"),
            "nexo:entity/equipment/humanoid/ruby"
        );
        assert_eq!(
            sprite("minecraft:diamond", "humanoid_leggings"),
            "entity/equipment/humanoid_leggings/diamond"
        );
        assert_eq!(sprite("gold", "humanoid"), "entity/equipment/humanoid/gold");
    }

    #[test]
    fn layers_read_every_kind_and_the_undyed_colour() {
        let v = serde_json::json!({"layers": {
            "humanoid": [{"texture": "nexo:ruby", "dyeable": {"color_when_undyed": 0x123456}}],
            "humanoid_leggings": [{"texture": "nexo:ruby"}],
        }});
        let kinds = layers("nexo:ruby", &v);
        assert_eq!(
            kinds["humanoid"],
            ("nexo:entity/equipment/humanoid/ruby".into(), Some(0x123456))
        );
        assert_eq!(
            kinds["humanoid_leggings"],
            ("nexo:entity/equipment/humanoid_leggings/ruby".into(), None)
        );
    }
}
