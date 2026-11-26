use super::*;
use crate::prelude::*;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use sunrise_mod_runtime::attachments::{self as runtime, index::AttachmentIndex};

#[derive(Default)]
struct Store {
    index: Arc<Mutex<AttachmentIndex<AttachmentHolder>>>,
    maps: HashMap<Holder, Attachments>,
    read_from: Option<Holder>,
}
impl Host for Store {}
crate::impl_host_areas!(Store, except(mod_state));
impl ModStateHost for Store {
    fn attachment_query(
        &self,
        key: &AttachmentCaptureKey<'_>,
    ) -> Result<AttachmentBatch, AttachmentError> {
        let (namespace, name) = key.id();
        Ok(runtime::AttachmentBatch::new(
            self.index.lock().unwrap().snapshot_named(namespace, name),
        )
        .into())
    }
    fn attachments(&mut self) -> &mut dyn AttachmentStore {
        self
    }
}
impl ServerHost for Store {}
crate::impl_server_host_areas!(Store, except(mod_state));
impl ModStateServerHost for Store {
    fn attachment_validate_write(
        &self,
        token: AttachmentWriteToken,
    ) -> Result<(), AttachmentRowError> {
        crate::__private::game::validate_attachment_write(&token, Some(&self.index))
    }
}
impl AttachmentHost for Store {
    fn validate_attachment(
        &self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<(), AttachmentError> {
        let _ = (namespace, name, sync);
        Ok(())
    }
    fn validate_attachment_key(
        &self,
        write: &AttachmentStoreKey<'_>,
    ) -> Result<(), AttachmentError> {
        crate::__private::validate_attachment_key(write)
    }
    fn attachments(
        &mut self,
        holder: Holder,
        access: AttachmentAccess,
    ) -> Result<&mut Attachments, AttachmentError> {
        let holder = if access == AttachmentAccess::Existing {
            self.read_from.unwrap_or(holder)
        } else {
            holder
        };
        self.maps
            .get_mut(&holder)
            .ok_or(AttachmentError::MissingHolder)
    }
}

#[test]
fn all_scoped_variants_write_and_remove_the_exact_storage_target() {
    let epoch = WorldEpoch::new(19);
    let level = LevelRef::new(epoch, DimensionKey::NETHER);
    let cases = [
        (
            AttachmentHolder::Entity(EntityRef::new(epoch, 41)),
            Holder::Entity(41),
        ),
        (
            AttachmentHolder::Player(crate::__private::game::player_ref(
                EntityRef::new(epoch, 83),
                123,
            )),
            Holder::Player(123),
        ),
        (
            AttachmentHolder::Chunk(ChunkRef::new(level, [-7, 12])),
            Holder::Chunk([-7, 12]),
        ),
        (
            AttachmentHolder::BlockEntity(crate::__private::game::block_entity_ref(
                level,
                [-2, 64, 7],
                29,
            )),
            Holder::BlockEntity([-2, 64, 7]),
        ),
        (AttachmentHolder::Level(level), Holder::Level),
    ];
    let key = crate::parts::install_attachment_fixture(
        "mapping",
        crate::AttachmentDescriptor::new("mapping:value", || 0u32),
    );
    let mut host = Store::default();
    for (scoped, target) in cases {
        let mut map = Attachments::default();
        key.descriptor().set(&mut map, target, 10).unwrap();
        map.bind_index(&host.index, scoped);
        host.maps.insert(target, map);
    }
    // These adjacent addresses catch writes shifted by one field as well as
    // writes that simply fail: none belongs to a captured scoped row.
    let decoys = [
        Holder::Entity(42),
        Holder::Player(124),
        Holder::Chunk([-6, 13]),
        Holder::BlockEntity([-1, 65, 8]),
    ];
    for target in decoys {
        let mut map = Attachments::default();
        key.descriptor().set(&mut map, target, 99).unwrap();
        host.maps.insert(target, map);
    }
    let batch = host.query_mut(&key).unwrap();
    let captured = || {
        let mut rows = batch.iter();
        std::iter::from_fn(|| rows.try_next().unwrap())
            .map(|row| {
                (
                    row.holder(),
                    row.with_value(Default::default(), |value| *value).unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    let original: Vec<_> = cases.iter().map(|(holder, _)| (*holder, 10)).collect();
    assert_eq!(captured(), original);
    for (scoped, target) in cases {
        let mut rows = batch.iter();
        let row = std::iter::from_fn(|| rows.try_next().unwrap())
            .find(|row| row.holder() == scoped)
            .unwrap();
        row.set(&mut host, 23).unwrap();
        assert_eq!(
            host.maps[&target]
                .entry("mapping", "value")
                .unwrap()
                .unwrap()
                .read_checked(key.descriptor()),
            Ok(&23)
        );
        assert_eq!(row.with_value(Default::default(), |value| *value), Ok(10));
        assert_eq!(row.remove(&mut host).unwrap(), Some(23));
        assert_eq!(captured(), original);
        assert!(
            host.maps[&target]
                .entry("mapping", "value")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            row.with_value(Default::default(), |value| *value).unwrap(),
            10
        );
    }
    for target in decoys {
        assert_eq!(
            host.maps[&target]
                .entry("mapping", "value")
                .unwrap()
                .unwrap()
                .read_checked(key.descriptor()),
            Ok(&99)
        );
    }
    assert!(
        host.query(&key)
            .unwrap()
            .iter()
            .try_next()
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_runtime_descriptor_cannot_read_or_discard_an_installed_keys_value() {
    let key = crate::parts::install_attachment_fixture(
        "mapping",
        AttachmentDescriptor::new("mapping:protected", || 0u32),
    );
    let mut host = Store::default();
    host.maps.insert(Holder::Level, Attachments::default());
    key.set(&mut host, Holder::Level, 73).unwrap();
    let forged = runtime::AttachmentType {
        namespace: "mapping",
        name: "protected",
        default: || 0u32,
        codec: None,
        copy_on_death: false,
        sync_codec: None,
    };
    assert_eq!(
        forged.get_existing(&mut host, Holder::Level),
        Err(AttachmentError::StaleKey)
    );
    assert_eq!(
        forged.remove(&mut host, Holder::Level),
        Err(AttachmentError::StaleKey)
    );
    assert_eq!(
        key.get_existing(&mut host, Holder::Level, |value| value.copied()),
        Ok(Some(73))
    );
}

#[test]
fn stale_key_is_refused_by_queries_and_retained_row_reads_and_writes() {
    let epoch = WorldEpoch::new(19);
    let level = LevelRef::new(epoch, DimensionKey::OVERWORLD);
    let scoped = AttachmentHolder::Level(level);
    let first = crate::parts::install_attachment_fixture(
        "mapping",
        crate::AttachmentDescriptor::new("mapping:stale", || 0u32),
    );
    let mut host = Store::default();
    let mut map = Attachments::default();
    first.descriptor().set(&mut map, Holder::Level, 17).unwrap();
    map.bind_index(&host.index, scoped);
    host.maps.insert(Holder::Level, map);

    let retained = host.query_mut(&first).unwrap();
    let row = retained.iter().try_next().unwrap().unwrap();
    assert_eq!(
        row.with_value(Default::default(), |value| *value).unwrap(),
        17
    );

    // Reinstall the same ID and Rust type: the prior key becomes stale for fresh queries
    // and retained row writes, while the freshly issued key keeps working.
    let second = crate::parts::install_attachment_fixture(
        "mapping",
        crate::AttachmentDescriptor::new("mapping:stale", || 0u32),
    );
    assert_eq!(
        row.with_value(Default::default(), |value| *value),
        Err(AttachmentRowError::Value(AttachmentError::StaleKey))
    );
    assert_eq!(host.query(&first).err(), Some(AttachmentError::StaleKey));
    assert_eq!(
        host.query_mut(&first).err(),
        Some(AttachmentError::StaleKey)
    );
    assert_eq!(host.capture(&first).err(), Some(AttachmentError::StaleKey));
    assert_eq!(
        row.set(&mut host, 99),
        Err(AttachmentRowError::Value(AttachmentError::StaleKey))
    );
    assert_eq!(
        row.remove(&mut host),
        Err(AttachmentRowError::Value(AttachmentError::StaleKey))
    );
    assert_eq!(
        host.query(&second)
            .unwrap()
            .get(Default::default(), scoped, |value| value.cloned()),
        Ok(Some(17))
    );
}

#[test]
fn retained_raw_query_uses_the_current_installations_decoder() {
    let first_codec = AttachmentCodec {
        encode: |_| Err(AttachmentError::Encode),
        decode: |_| Ok(17u32),
    };
    let first = crate::parts::install_attachment_fixture(
        "mapping",
        AttachmentDescriptor::new("mapping:raw", || 0u32).sync(first_codec),
    );
    let scoped =
        AttachmentHolder::Level(LevelRef::new(WorldEpoch::new(19), DimensionKey::OVERWORLD));
    let host = Store::default();
    let mut map = Attachments::default();
    map.apply_sync(runtime::AttachmentSync {
        namespace: "mapping".into(),
        name: "raw".into(),
        value: Some(Compound::new()),
    });
    map.bind_index(&host.index, scoped);
    let retained = host.capture(&first).unwrap();
    let stale = retained.clone().query(&first).unwrap();
    let mut cursor = stale.iter();

    let second = crate::parts::install_attachment_fixture(
        "mapping",
        AttachmentDescriptor::new("mapping:raw", || 0u32).sync(AttachmentCodec {
            decode: |_| Ok(38),
            ..first_codec
        }),
    );
    assert_eq!(
        stale.get(Default::default(), scoped, |value| value.cloned()),
        Err(AttachmentRowError::Value(AttachmentError::StaleKey))
    );
    assert_eq!(
        cursor.try_next().err(),
        Some(AttachmentRowError::Value(AttachmentError::StaleKey))
    );
    assert_eq!(
        host.query(&second)
            .unwrap()
            .get(Default::default(), scoped, |value| value.cloned()),
        Ok(Some(38))
    );
    assert_eq!(
        retained
            .query(&second)
            .unwrap()
            .iter()
            .try_next()
            .unwrap()
            .unwrap()
            .with_value(Default::default(), |value| *value)
            .unwrap(),
        38
    );
}

#[test]
fn public_store_only_writes_the_issued_descriptor_type_and_policy() {
    const CODEC: AttachmentCodec<u32> = AttachmentCodec {
        encode: |value| Ok([("projection".into(), Tag::Int(*value as i32 + 100))].into()),
        decode: |_| Err(AttachmentError::Decode),
    };
    let key = crate::parts::install_attachment_fixture(
        "mapping",
        AttachmentDescriptor::new("mapping:sync", || 0u32).sync(CODEC),
    );
    let mut host = Store::default();
    host.maps.insert(Holder::Level, Attachments::default());
    let imposter = runtime::AttachmentType {
        namespace: "mapping",
        name: "sync",
        default: || 0u32,
        codec: Some(CODEC),
        copy_on_death: false,
        sync_codec: Some(CODEC),
    };
    assert_eq!(
        imposter.set(ModStateHost::attachments(&mut host), Holder::Level, 8),
        Err(AttachmentError::StaleKey)
    );
    let wrong_type = runtime::AttachmentType {
        namespace: "mapping",
        name: "sync",
        default: || false,
        codec: None,
        copy_on_death: false,
        sync_codec: None,
    };
    assert_eq!(
        wrong_type.set(ModStateHost::attachments(&mut host), Holder::Level, true),
        Err(AttachmentError::StaleKey)
    );
    assert_eq!(
        key.get_existing(&mut host, Holder::Level, |value| value.cloned()),
        Ok(None)
    );

    key.set(&mut host, Holder::Level, 9).unwrap();
    let map = host.maps.get_mut(&Holder::Level).unwrap();
    assert!(map.try_to_nbt().unwrap().is_empty());
    let changes = map.take_sync_changes(|namespace, name, input| {
        crate::__private::attachment_sync_projection()(namespace, name, input).unwrap()
    });
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0].value,
        Some([("projection".into(), Tag::Int(109))].into())
    );
}

#[test]
fn a_captured_bucket_refuses_another_live_key_of_the_same_type() {
    let codec = AttachmentCodec {
        encode: |_| Err(AttachmentError::Encode),
        decode: |_| Ok(17u32),
    };
    let (fuel, fuel_key) = crate::parts::registered_attachment_fixture(
        "mapping",
        AttachmentDescriptor::new("mapping:fuel", || 0u32).sync(codec),
    );
    let (charge, charge_key) = crate::parts::registered_attachment_fixture(
        "mapping",
        AttachmentDescriptor::new("mapping:charge", || 0u32).sync(AttachmentCodec {
            decode: |_| Ok(99),
            ..codec
        }),
    );
    crate::parts::install_input(crate::parts::installation::InstalledInput {
        attachment_types: HashMap::from([(
            "mapping",
            HashMap::from([("fuel", fuel), ("charge", charge)]),
        )]),
        ..Default::default()
    });
    let holder =
        AttachmentHolder::Level(LevelRef::new(WorldEpoch::new(19), DimensionKey::OVERWORLD));
    let host = Store::default();
    let mut values = Attachments::default();
    values.apply_sync(runtime::AttachmentSync {
        namespace: "mapping".into(),
        name: "fuel".into(),
        value: Some(Compound::new()),
    });
    values.bind_index(&host.index, holder);
    let batch = host.capture(&fuel_key).unwrap();
    assert_eq!(
        batch.query(&charge_key).err(),
        Some(AttachmentError::TypeMismatch)
    );
    assert_eq!(
        host.query(&fuel_key)
            .unwrap()
            .get(Default::default(), holder, |value| value.cloned()),
        Ok(Some(17))
    );
}

fn counter_on_disk(n: i32) -> Compound {
    let payload = [("value".into(), Tag::Int(n))].into();
    let envelope = [("value".into(), Tag::Compound(payload))].into();
    let names = [("counter".into(), Tag::Compound(envelope))].into();
    [("mapping".into(), Tag::Compound(names))].into()
}

#[test]
fn read_loans_project_nonclone_values_from_the_live_holder_and_its_snapshot() {
    struct Counter(i32);
    let key = crate::parts::install_attachment_fixture(
        "mapping",
        AttachmentDescriptor::new("mapping:counter", || Counter(7)).serialize(AttachmentCodec {
            encode: |n| Ok([("value".into(), Tag::Int(n.0))].into()),
            decode: |data| match data.get(&"value".into()) {
                Some(Tag::Int(n)) => Ok(Counter(*n)),
                _ => Err(AttachmentError::Decode),
            },
        }),
    );
    let level =
        AttachmentHolder::Level(LevelRef::new(WorldEpoch::new(19), DimensionKey::OVERWORLD));
    let mut host = Store::default();
    let mut disk = Attachments::from_nbt(counter_on_disk(11));
    disk.bind_index(&host.index, level);
    host.maps.insert(Holder::Level, disk);
    host.maps.insert(Holder::Entity(1), Attachments::default());

    assert_eq!(
        key.get_existing(&mut host, Holder::Entity(1), |value| value.map(|n| n.0)),
        Ok(None)
    );
    assert!(!key.has(&mut host, Holder::Entity(1)).unwrap());
    assert_eq!(key.get(&mut host, Holder::Entity(1), |n| n.0), Ok(7));
    assert!(key.has(&mut host, Holder::Entity(1)).unwrap());
    let project = |value: Option<&Counter>| value.map(|n| n.0);
    assert_eq!(
        key.with_decoded(&mut host, Holder::Level, project),
        Ok(None)
    );
    assert_eq!(
        key.get_existing(&mut host, Holder::Level, project),
        Ok(Some(11))
    );
    assert_eq!(
        key.with_decoded(&mut host, Holder::Level, project),
        Ok(Some(11))
    );

    let capture = host.query(&key).unwrap();
    key.set(&mut host, Holder::Level, Counter(13)).unwrap();
    assert_eq!(key.get(&mut host, Holder::Level, |n| n.0), Ok(13));
    assert_eq!(
        capture.get(Default::default(), level, project),
        Ok(Some(11))
    );
}

#[test]
fn an_issued_promotion_cannot_retarget_a_binding_or_write_into_a_malformed_namespace() {
    let key = crate::parts::install_attachment_fixture(
        "mapping",
        AttachmentDescriptor::new("mapping:counter", || 0i32).serialize(AttachmentCodec {
            encode: |n| Ok([("value".into(), Tag::Int(*n))].into()),
            decode: |data| match data.get(&"value".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
    );
    let (from, to) = (Holder::Entity(1), Holder::Entity(2));
    let mut host = Store {
        maps: HashMap::from([
            (from, Attachments::from_nbt(counter_on_disk(99))),
            (to, Attachments::from_nbt(counter_on_disk(1))),
        ]),
        read_from: Some(from),
        ..Default::default()
    };
    assert_eq!(
        key.get_existing(&mut host, to, |value| value.cloned()),
        Err(AttachmentError::Decode)
    );
    assert_eq!(host.maps[&to].try_to_nbt().unwrap(), counter_on_disk(1));
    host.read_from = None;
    assert_eq!(
        key.get_existing(&mut host, to, |value| value.cloned()),
        Ok(Some(1))
    );

    let malformed: Compound = [("mapping".into(), Tag::Int(3))].into();
    host.maps
        .insert(to, Attachments::from_nbt(malformed.clone()));
    host.read_from = Some(from);
    assert_eq!(
        key.set(&mut host, to, 42),
        Err(AttachmentError::InvalidNamespace)
    );
    assert_eq!(host.maps[&to].try_to_nbt().unwrap(), malformed);
}

#[test]
#[ignore = "daily"]
fn a_refused_promotion_or_default_drops_only_one_value() {
    const NAME: &str =
        "attachment_queries::tests::a_refused_promotion_or_default_drops_only_one_value";
    const CHILD: &str = "SUNRISE_ATTACHMENT_PROMOTION_DROP_CHILD";
    // libtest exits 0 even when --exact matched nothing. Only the child that
    // completed its assertions, including drops, returns this code.
    const ASSERTIONS_PASSED: i32 = 42;
    if std::env::var_os(CHILD).is_some() {
        assert_single_drop_after_attachment_refusals();
        std::process::exit(ASSERTIONS_PASSED);
    }
    let output = crate::testing::fresh_case_command(NAME, CHILD, "refusals")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(ASSERTIONS_PASSED),
        "{NAME}: {:?}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_single_drop_after_attachment_refusals() {
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    static CLONES: AtomicUsize = AtomicUsize::new(0);
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct PanickingValue;
    impl Clone for PanickingValue {
        fn clone(&self) -> Self {
            CLONES.fetch_add(1, Relaxed);
            Self
        }
    }
    impl Drop for PanickingValue {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Relaxed);
            panic!("refused attachment value destructor");
        }
    }
    let key = crate::parts::install_attachment_fixture(
        "mapping",
        AttachmentDescriptor::new("mapping:counter", || PanickingValue).serialize(
            AttachmentCodec {
                encode: |_| Ok(Compound::new()),
                decode: |_| Ok(PanickingValue),
            },
        ),
    );
    let (from, to) = (Holder::Entity(1), Holder::Entity(2));
    let mut host = Store {
        maps: HashMap::from([
            (from, Attachments::from_nbt(counter_on_disk(99))),
            (to, Attachments::from_nbt(counter_on_disk(1))),
        ]),
        read_from: Some(from),
        ..Default::default()
    };
    let run = |host: &mut Store, default| {
        let result = crate::runtime::contain_callback(
            || {
                if default {
                    key.get(host, to, Clone::clone).err()
                } else {
                    key.get_existing(host, to, |value| value.cloned()).err()
                }
            },
            |_| {},
        );
        assert!(
            result.is_none(),
            "the single destructor panic should reach the callback fence"
        );
    };
    run(&mut host, false);
    assert_eq!(DROPS.load(Relaxed), 1);
    assert_eq!(host.maps[&to].try_to_nbt().unwrap(), counter_on_disk(1));

    // No entry is visible to the read adapter, but the real store must refuse the
    // default because its destination preserves a malformed namespace.
    let malformed: Compound = [("mapping".into(), Tag::Int(3))].into();
    host.maps.insert(from, Attachments::default());
    host.maps
        .insert(to, Attachments::from_nbt(malformed.clone()));
    run(&mut host, true);
    assert_eq!(DROPS.load(Relaxed), 2);
    assert_eq!(CLONES.load(Relaxed), 0);
    assert_eq!(host.maps[&to].try_to_nbt().unwrap(), malformed);
}
