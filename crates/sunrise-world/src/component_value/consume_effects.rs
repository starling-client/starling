//! Local consume-effect codecs, keyed by name independently of any wire ID assignment.
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};
use sunrise_core::nbt::{Compound, Tag};

/// An admitted effect: the codec value's canonical saved map and its typed Wire form.
#[derive(Clone, Debug)]
pub struct Admitted {
    pub name: Arc<str>,
    pub saved: Arc<Tag>,
    pub wire: Arc<[u8]>,
    // A preserved template can still hold the original map rather than its canonical projection.
    source: Option<Arc<Tag>>,
}

impl PartialEq for Admitted {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.saved == other.saved && self.wire == other.wire
    }
}

type Load = dyn Fn(&Compound) -> Result<(Compound, Vec<u8>), String> + Send + Sync;
type Read = dyn Fn(&[u8], bool) -> Result<(usize, Option<Compound>), String> + Send + Sync;

/// The type-erased adapters for a mod's map and Wire codecs.
pub struct Type {
    pub schema: u64,
    pub load: Box<Load>,
    pub read: Box<Read>,
}

type Types = BTreeMap<Arc<str>, Type>;
static TYPES: OnceLock<Types> = OnceLock::new();

#[cfg(feature = "test-support")]
thread_local! {
    static TEST_TYPES: std::cell::Cell<Option<Option<&'static Types>>> = const { std::cell::Cell::new(None) };
}

/// Run fixture installation and its consumers with a thread-local registry.
/// Resolved one-shot fixtures leak their codec table like the test mod snapshot.
#[cfg(feature = "test-support")]
pub fn with_test_registry<T>(run: impl FnOnce() -> T) -> T {
    struct Restore(Option<Option<&'static Types>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_TYPES.set(self.0);
        }
    }
    let _restore = Restore(TEST_TYPES.replace(Some(None)));
    run()
}

pub fn install(types: Types) -> Result<(), String> {
    #[cfg(feature = "test-support")]
    if let Some(installed) = TEST_TYPES.get() {
        if installed.is_some() {
            return Err("test consume effect types already installed".into());
        }
        TEST_TYPES.set(Some(Some(Box::leak(Box::new(types)))));
        return Ok(());
    }
    TYPES
        .set(types)
        .map_err(|_| "consume effect types already installed".into())
}

fn installed() -> Option<&'static Types> {
    #[cfg(feature = "test-support")]
    if let Some(types) = TEST_TYPES.get() {
        return types;
    }
    TYPES.get()
}

pub fn entries() -> impl Iterator<Item = (&'static Arc<str>, &'static Type)> {
    installed().into_iter().flatten()
}

pub fn by_name(name: &str) -> Option<(&'static Arc<str>, &'static Type)> {
    installed()?.get_key_value(name)
}

impl Admitted {
    pub fn new(name: &Arc<str>, mut fields: Compound, wire: Arc<[u8]>) -> Self {
        fields.insert("type".into(), Tag::String(name.as_ref().into()));
        Self {
            name: name.clone(),
            saved: Arc::new(Tag::Compound(fields)),
            wire,
            source: None,
        }
    }

    pub fn matches_tag(&self, tag: &Tag) -> bool {
        self.saved.as_ref() == tag || self.source.as_deref() == Some(tag)
    }
}

/// Decode a registered map value when its containing stack is materialized.
/// `None` means the key is not registered, not that its registered codec refused the value.
pub fn admit(name: &str, fields: &Compound) -> Result<Option<Admitted>, String> {
    let Some((name, ty)) = by_name(name) else {
        return Ok(None);
    };
    let mut input = fields.clone();
    input.remove(&"type".into());
    let (saved, wire) = (ty.load)(&input)?;
    let mut effect = Admitted::new(name, saved, wire.into());
    if matches!(effect.saved.as_ref(), Tag::Compound(saved) if saved != fields) {
        effect.source = Some(Arc::new(Tag::Compound(fields.clone())));
    }
    Ok(Some(effect))
}
