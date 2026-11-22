use super::*;
use crate::prelude::*;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use sunrise_mod_runtime::attachments::{self as runtime, index::AttachmentIndex};

static VALUE: AttachmentType<u32> = AttachmentType {
    namespace: "mapping",
    name: "value",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};
#[derive(Default)]
struct Store {
    index: Arc<Mutex<AttachmentIndex<AttachmentHolder>>>,
    maps: HashMap<Holder, Attachments>,
}
impl Host for Store {}
crate::impl_host_areas!(Store, except(mod_state));
impl ModStateHost for Store {
    fn attachment_query(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<AttachmentBatch, AttachmentError> {
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
    fn attachments(
        &mut self,
        holder: Holder,
        _: AttachmentAccess,
    ) -> Result<&mut Attachments, AttachmentError> {
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
    crate::parts::install_input(crate::parts::installation::InstalledInput {
        attachment_types: HashMap::from([(
            "mapping".into(),
            HashMap::from([("value".into(), false)]),
        )]),
        ..Default::default()
    });
    let mut host = Store::default();
    for (scoped, target) in cases {
        let mut map = Attachments::default();
        VALUE.set(&mut map, target, 10).unwrap();
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
        VALUE.set(&mut map, target, 99).unwrap();
        host.maps.insert(target, map);
    }
    let batch = host.query_mut(&VALUE).unwrap();
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
                .read(&VALUE),
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
                .read(&VALUE),
            Ok(&99)
        );
    }
    assert!(
        host.query(&VALUE)
            .unwrap()
            .iter()
            .try_next()
            .unwrap()
            .is_none()
    );
}
