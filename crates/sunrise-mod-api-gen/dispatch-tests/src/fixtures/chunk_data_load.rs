//! Chunk disk edits use the same per-listener panic boundary as other borrowed outputs.
mod stubs {
    pub(super) trait ChunkDiskHost {}
    #[derive(Clone)]
    pub(super) struct ChunkDiskData {
        pub value: i32,
    }
}

impl ChunkDiskHost for TestHost {}

#[test]
fn a_panicking_listener_cannot_discard_other_chunk_edits() {
    struct Listener {
        input: i32,
        output: i32,
        unwind: bool,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn chunk_data_load(&self, _: &dyn ChunkDiskHost, event: &mut ChunkDataLoadEvent<'_>) {
            assert_eq!(event.data.value, self.input);
            event.data.value = self.output;
            assert!(!self.unwind, "tentative chunk edit");
        }
    }

    install(
        [
            ("disk_first", 7, 8, false),
            ("disk_panic", 8, 99, true),
            ("disk_last", 8, 9, false),
        ]
        .into_iter()
        .map(|(id, input, output, unwind)| {
            ModEntry::new(
                id,
                Box::new(Listener {
                    input,
                    output,
                    unwind,
                }),
            )
        })
        .collect(),
    );
    let mut job = ChunkDiskData { value: 7 };
    mod_api::dispatch_chunk_data_load(&TestHost, &mut job);
    assert_eq!(job.value, 9);
    assert!(runtime::poisoned(HookId::ChunkDataLoad, "disk_panic"));
    assert!(!runtime::poisoned(HookId::ChunkDataLoad, "disk_first"));
    assert!(!runtime::poisoned(HookId::ChunkDataLoad, "disk_last"));
}
