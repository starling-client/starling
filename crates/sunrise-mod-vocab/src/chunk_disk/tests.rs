use super::*;
use crate::testing::TestReader;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_core::nbt::{Limits, NamedTag, write_named};
use sunrise_mod_runtime::attachments::ATTACHMENTS_KEY;

const VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "disk_fixture",
    name: "value",
    default: || 0,
    codec: Some(sunrise_mod_runtime::attachments::AttachmentCodec {
        encode: |value| Ok(Compound::from_iter([("value".into(), Tag::Int(*value))])),
        decode: |tag| match tag.get(&"value".into()) {
            Some(Tag::Int(value)) => Ok(*value),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

fn store(value: i32) -> Attachments {
    let mut store = Attachments::default();
    VALUE.set(&mut store, Holder::Level, value).unwrap();
    store
}
fn context(phase: ChunkDiskPhase) -> ChunkDiskContext {
    ChunkDiskContext::new(
        WorldEpoch::new(17),
        Identifier::parse("minecraft:overworld").unwrap(),
        [2, -3],
        ChunkDiskKind::Full,
        phase,
    )
}
fn read(entries: &Compound) -> i32 {
    VALUE
        .get(&mut Attachments::from_nbt(entries.clone()), Holder::Level)
        .unwrap()
}
fn run(job: &ChunkDiskJob) -> Result<Compound, ChunkDiskFailure> {
    let reader = TestReader::default();
    job.run(&OwningWorldgen::new(&reader, &|_, _, _| None))
        .clone()
}
struct Listener {
    calls: Arc<AtomicUsize>,
    panics: bool,
}
#[crate::__subscriptions]
impl Hooks for Listener {
    fn chunk_data_save(&self, host: &dyn ChunkDiskHost, event: &mut ChunkDataSaveEvent<'_>) {
        assert!(host.backend().is_none());
        assert_eq!(event.data.get(&VALUE).unwrap(), 7);
        event.data.set(&VALUE, 8).unwrap();
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.panics {
            panic!("fixture unwind after tentative write");
        }
    }
    fn chunk_data_load(&self, _: &dyn ChunkDiskHost, event: &mut ChunkDataLoadEvent<'_>) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let value = event.data.get(&VALUE).unwrap();
        event.data.set(&VALUE, value + 1).unwrap();
    }
}
fn with_listener(apply: impl FnOnce(Arc<AtomicUsize>)) {
    let calls = Arc::new(AtomicUsize::new(0));
    testing::with_dispatch(
        vec![ModEntry::new(
            "disk_fixture",
            Box::new(Listener {
                calls: calls.clone(),
                panics: false,
            }),
        )]
        .into(),
        || apply(calls),
    );
}

#[test]
fn host_borrows_the_captured_context() {
    let captured = context(ChunkDiskPhase::Save);
    let reader = TestReader::default();
    let resources = OwningWorldgen::new(&reader, &|_, _, _| None);
    let host: &dyn ChunkDiskHost = &OwningChunkDisk::new(&captured, &resources);
    assert!(std::ptr::eq(host.job(), &captured));
}

#[test]
fn capture_7_callback_8_live_9() {
    with_listener(|calls| {
        let mut live = store(7);
        let job = ChunkDiskJob::capture(&context(ChunkDiskPhase::Save), &live).unwrap();
        VALUE.set(&mut live, Holder::Level, 9).unwrap();
        assert_eq!(read(&run(&job).unwrap()), 8);
        assert_eq!(VALUE.get(&mut live, Holder::Level).unwrap(), 9);
        assert_eq!(read(&run(&job).unwrap()), 8);
        assert_eq!(read(job.captured()), 7);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    });
}

#[test]
fn load_disk_8_callback_increment_publishes_9_once() {
    with_listener(|calls| {
        let job = ChunkDiskJob::capture(&context(ChunkDiskPhase::Load), &store(8)).unwrap();
        assert_eq!(read(&run(&job).unwrap()), 9);
        assert_eq!(read(&run(&job).unwrap()), 9);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    });
}

#[test]
fn an_unwinding_disk_listener_refuses_publication_but_does_not_stop_the_walk() {
    let a = Arc::new(AtomicUsize::new(0));
    let b = Arc::new(AtomicUsize::new(0));
    testing::with_dispatch(
        vec![
            ModEntry::new(
                "disk_a",
                Box::new(Listener {
                    calls: a.clone(),
                    panics: true,
                }),
            ),
            ModEntry::new(
                "disk_b",
                Box::new(Listener {
                    calls: b.clone(),
                    panics: false,
                }),
            ),
        ]
        .into(),
        || {
            let failed = ChunkDiskJob::capture(&context(ChunkDiskPhase::Save), &store(7)).unwrap();
            let refusal = run(&failed).unwrap_err();
            assert_eq!(refusal.mod_id, "disk_a");
            assert_eq!(refusal.reason, "native listener unwound");
            assert_eq!(read(failed.captured()), 7);
            assert_eq!(b.load(Ordering::Relaxed), 1, "B runs on the failed job");
            assert_eq!(run(&failed).unwrap_err(), refusal);
            assert_eq!(b.load(Ordering::Relaxed), 1, "the failed job is complete");
            let next = ChunkDiskJob::capture(&context(ChunkDiskPhase::Save), &store(7)).unwrap();
            assert_eq!(read(&run(&next).unwrap()), 8, "only B runs and publishes");
            assert_eq!(a.load(Ordering::Relaxed), 1);
            assert_eq!(b.load(Ordering::Relaxed), 2);
            assert_eq!(
                crate::__private::take_hook_panics(),
                [crate::__private::HookPanicReport {
                    mod_id: "disk_a",
                    hook: "chunk_data_save",
                }]
            );
        },
    );
}

#[test]
fn a_disk_codec_failure_remains_structured_and_does_not_disable_the_listener() {
    struct BrokenCodec;
    #[crate::__subscriptions]
    impl Hooks for BrokenCodec {
        fn chunk_data_save(&self, _: &dyn ChunkDiskHost, event: &mut ChunkDataSaveEvent<'_>) {
            let broken = AttachmentType {
                codec: Some(sunrise_mod_runtime::attachments::AttachmentCodec {
                    encode: |_| Err(AttachmentError::Encode),
                    decode: |_| Ok(7),
                }),
                ..VALUE
            };
            let _ = event.data.set(&broken, 8);
        }
    }
    testing::with_dispatch(
        vec![ModEntry::new("broken_codec", Box::new(BrokenCodec))].into(),
        || {
            for _ in 0..2 {
                let job = ChunkDiskJob::capture(&context(ChunkDiskPhase::Save), &store(7)).unwrap();
                let refusal = run(&job).unwrap_err();
                assert_eq!(refusal.mod_id, "broken_codec");
                assert_eq!(refusal.key, "disk_fixture:value");
                assert_eq!(refusal.reason, "Encode");
            }
            assert!(crate::__private::take_hook_panics().is_empty());
        },
    );
}

#[test]
fn disk_operations_report_their_own_error_without_latching_and_later_mods_run() {
    struct Refuses;
    #[crate::__subscriptions]
    impl Hooks for Refuses {
        fn chunk_data_save(&self, _: &dyn ChunkDiskHost, event: &mut ChunkDataSaveEvent<'_>) {
            let first = AttachmentType {
                name: "first",
                codec: None,
                ..VALUE
            };
            let second = AttachmentType {
                name: "second",
                codec: None,
                ..VALUE
            };
            let first = event.data.get(&first).unwrap_err();
            assert_eq!(first.key, "disk_fixture:first");
            assert_eq!(first.mod_id, "refuses");
            let second = event.data.remove(&second).unwrap_err();
            assert_eq!(second.key, "disk_fixture:second");
            assert_eq!(second.mod_id, "refuses");
            assert_eq!(event.data.failure(), Some(&first));
            event.data.set(&VALUE, 99).unwrap();
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    testing::with_dispatch(
        vec![
            ModEntry::new("refuses", Box::new(Refuses)),
            ModEntry::new(
                "later",
                Box::new(Listener {
                    calls: calls.clone(),
                    panics: false,
                }),
            ),
        ]
        .into(),
        || {
            let job = ChunkDiskJob::capture(&context(ChunkDiskPhase::Save), &store(7)).unwrap();
            let failure = run(&job).unwrap_err();
            assert_eq!(failure.key, "disk_fixture:first");
            assert_eq!(failure.mod_id, "refuses");
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            assert!(crate::__private::take_hook_panics().is_empty());
        },
    );
}

fn opaque(value: Tag) -> Compound {
    Compound::from_iter([(
        "unknown".into(),
        Tag::Compound(Compound::from_iter([(
            "value".into(),
            Tag::Compound(Compound::from_iter([("value".into(), value)])),
        )])),
    )])
}

#[test]
fn mixed_list_value_budget_includes_the_name_and_both_wrappers() {
    let mixed = |len| Tag::List(vec![Tag::Byte(1), Tag::ByteArray(vec![0; len])]);
    // Eight bytes for the named tag, five for the list header, eight for
    // the two wrappers, one for the byte and four for the array length.
    let value = mixed(65_510);
    let disk = write_named(
        &NamedTag {
            name: "value".into(),
            tag: value.clone(),
        },
        Limits::saved(),
    )
    .unwrap();
    assert_eq!(disk.len(), 65_536);
    assert!(ChunkDiskData::from_nbt(opaque(value)).is_ok());
    let refusal = ChunkDiskData::from_nbt(opaque(mixed(65_511))).unwrap_err();
    assert_eq!(refusal.key, "unknown:value");
    assert_eq!(refusal.reason, "encoded attachment exceeds 65536 bytes");
}

#[test]
fn value_depth_budget_includes_the_mixed_list_wrapper() {
    let mut value = Tag::List(vec![Tag::Byte(1), Tag::Int(2)]);
    for _ in 0..62 {
        value = Tag::List(vec![value]);
    }
    assert!(ChunkDiskData::from_nbt(opaque(value.clone())).is_ok());
    let refusal = ChunkDiskData::from_nbt(opaque(Tag::List(vec![value]))).unwrap_err();
    assert_eq!(refusal.reason, "attachment depth exceeds 64");
}

#[test]
fn chunk_budget_counts_framing_and_modified_utf8_for_empty_namespaces() {
    let empty = || Tag::Compound(Compound::new());
    let mut entries: Compound = ('a'..='o')
        .map(|c| (c.to_string().repeat(65_535).as_str().into(), empty()))
        .collect();
    // The 23-byte attachment frame and 15 namespaces at 65,539 bytes each
    // leave 65,468 bytes. The remaining namespace, envelope and value cost
    // 5 + 5 + 12 bytes before the array, and the last empty namespace costs 20.
    entries.insert(
        "x".into(),
        Tag::compound([(
            "v",
            Tag::compound([("value", Tag::ByteArray(vec![0; 65_426]))]),
        )]),
    );
    let mut last = "\0".repeat(8);
    entries.insert(last.as_str().into(), empty());
    let disk = write_named(
        &NamedTag {
            name: ATTACHMENTS_KEY.into(),
            tag: Tag::Compound(entries.clone()),
        },
        Limits::saved(),
    )
    .unwrap();
    assert_eq!(disk.len(), 1_048_576);
    assert_eq!(
        ChunkDiskData::from_nbt(entries.clone())
            .unwrap()
            .finish()
            .unwrap(),
        entries
    );
    entries.remove(&last.as_str().into());
    last.push('a');
    entries.insert(last.as_str().into(), empty());
    let refusal = ChunkDiskData::from_nbt(entries).unwrap_err();
    assert_eq!(refusal.reason, "chunk attachments exceed 1048576 bytes");
}

#[test]
fn codec_output_that_would_truncate_on_disk_refuses_the_job() {
    let long_string = AttachmentType {
        codec: Some(sunrise_mod_runtime::attachments::AttachmentCodec {
            encode: |_| {
                Ok(Compound::from_iter([(
                    "text".into(),
                    Tag::String("界".repeat(21_846).as_str().into()),
                )]))
            },
            decode: |_| Ok(7),
        }),
        ..VALUE
    };
    let mut data = ChunkDiskData::from_nbt(store(7).to_nbt()).unwrap();
    let refusal = data.set(&long_string, 8).unwrap_err();
    assert_eq!(refusal.key, "disk_fixture:value");
    assert_eq!(refusal.reason, "modified UTF-8 exceeds 65535 bytes");
    assert_eq!(data.finish().unwrap_err(), refusal);
}

#[test]
fn unknown_attachment_survives_typed_edit() {
    let mut data = ChunkDiskData::from_nbt(opaque(Tag::Int(7))).unwrap();
    data.set(&VALUE, 8).unwrap();
    let result = data.finish().unwrap();
    assert_eq!(
        result.get(&"unknown".into()),
        opaque(Tag::Int(7)).get(&"unknown".into())
    );
}

#[test]
fn zero_subscribers_over_100_saves_creates_no_jobs_or_callbacks() {
    testing::with_dispatch(Vec::new().into(), || {
        let snapshot = store(7);
        let context = context(ChunkDiskPhase::Save);
        for _ in 0..100 {
            assert!(ChunkDiskJob::capture(&context, &snapshot).is_none());
        }
    });
}

#[test]
fn local_transaction_abort_preserves_prior_snapshot() {
    let mut data = ChunkDiskData::from_nbt(store(7).to_nbt()).unwrap();
    let result: Result<(), ()> = data.transaction(|data| {
        data.set(&VALUE, 8).unwrap();
        Err(())
    });
    assert_eq!(result, Err(()));
    assert_eq!(read(&data.finish().unwrap()), 7);
}

#[test]
fn type_without_persistence_codec_refuses_naming_type_and_capability() {
    let unsaved = AttachmentType {
        codec: None,
        ..VALUE
    };
    let mut data = ChunkDiskData::from_nbt(store(7).to_nbt()).unwrap();
    let refusal = data.get(&unsaved).unwrap_err();
    assert_eq!(refusal.key, "disk_fixture:value");
    assert_eq!(refusal.reason, "missing persistence codec");
    // The refusal poisons the whole job even if the listener ignores it.
    assert_eq!(data.finish().unwrap_err(), refusal);
}
