//! Immutable biome identities and resolved holder sets for world predicates.
use super::{DecodeError, biome::BiomeId, density::identifier};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
pub struct BiomePredicateBindings {
    names: BTreeMap<String, BiomeId>,
    tags: BTreeMap<String, Arc<BTreeSet<BiomeId>>>,
}
impl BiomePredicateBindings {
    /// IDs must belong to the same registry snapshot as the world's biome query.
    pub fn new(
        names: &BTreeMap<String, BiomeId>,
        tags: &BTreeMap<String, Vec<String>>,
    ) -> Result<Self, DecodeError> {
        let mut canonical = BTreeMap::new();
        let mut ids = BTreeSet::new();
        for (name, id) in names {
            if canonical.insert(identifier(name)?, *id).is_some() || !ids.insert(*id) {
                return Err(DecodeError::Field("duplicate predicate biome identity"));
            }
        }
        let mut bound_tags = BTreeMap::new();
        for (name, entries) in tags {
            let values = entries
                .iter()
                .map(|entry| {
                    canonical
                        .get(&identifier(entry)?)
                        .copied()
                        .ok_or_else(|| DecodeError::Registry(format!("unknown biome {entry}")))
                })
                .collect::<Result<BTreeSet<_>, _>>()?;
            if bound_tags
                .insert(super::tags::identifier(name)?, Arc::new(values))
                .is_some()
            {
                return Err(DecodeError::Field("duplicate predicate biome tag"));
            }
        }
        Ok(Self {
            names: canonical,
            tags: bound_tags,
        })
    }
    pub fn resolve(&self, json: &str) -> Result<Arc<BTreeSet<BiomeId>>, DecodeError> {
        if let Ok(name) = serde_json::from_str::<String>(json) {
            if let Some(tag) = name.strip_prefix('#') {
                return self
                    .tags
                    .get(&super::tags::identifier(tag)?)
                    .cloned()
                    .ok_or_else(|| DecodeError::Registry(format!("unknown biome tag {tag}")));
            }
            return Ok(Arc::new(BTreeSet::from([self.biome(&name)?])));
        }
        let names: Vec<String> = serde_json::from_str(json)?;
        Ok(Arc::new(
            names
                .iter()
                .map(|name| self.biome(name))
                .collect::<Result<_, _>>()?,
        ))
    }
    fn biome(&self, name: &str) -> Result<BiomeId, DecodeError> {
        self.names
            .get(&identifier(name)?)
            .copied()
            .ok_or_else(|| DecodeError::Registry(format!("unknown biome {name}")))
    }
}

#[cfg(test)]
mod ceiling_tests {
    use super::*;
    #[test]
    fn biome_registry_ceiling() {
        let names = (0..8193)
            .map(|i| (format!("test:b{i}"), BiomeId(i)))
            .collect();
        let tags = (0..8193)
            .map(|i| (format!("test:t{i}"), vec!["test:b0".into()]))
            .collect();
        let bindings = BiomePredicateBindings::new(&names, &tags).unwrap();
        assert_eq!(bindings.resolve(r##""#test:t8192""##).unwrap().len(), 1);
        assert_eq!(bindings.resolve(r#""test:b8192""#).unwrap().len(), 1);
    }
    #[test]
    fn biome_tag_ceiling() {
        let names = BTreeMap::from([("test:b".into(), BiomeId(0))]);
        let tags = BTreeMap::from([("test:t".into(), vec!["test:b".into(); 8193])]);
        let bindings = BiomePredicateBindings::new(&names, &tags).unwrap();
        assert_eq!(bindings.resolve(r##""#test:t""##).unwrap().len(), 1);
    }
    #[test]
    fn biome_holder_ceiling() {
        let names = BTreeMap::from([("test:b".into(), BiomeId(0))]);
        let bindings = BiomePredicateBindings::new(&names, &BTreeMap::new()).unwrap();
        let json = serde_json::to_string(&vec!["test:b"; 8193]).unwrap();
        assert_eq!(bindings.resolve(&json).unwrap().len(), 1);
    }
}
