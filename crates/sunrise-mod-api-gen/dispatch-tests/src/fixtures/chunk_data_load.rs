//! Chunk dispatch keeps job completion separate from each listener's proposal.
mod stubs {
    pub(super) trait ChunkDiskHost {}
    #[derive(Clone, Debug, Default)]
    pub(super) struct ChunkDiskFailure {
        pub mod_id: String,
        pub reason: &'static str,
    }
    #[derive(Clone, Default)]
    pub(super) struct ChunkDiskData {
        pub value: i32,
        pub failure: Option<ChunkDiskFailure>,
    }
    impl ChunkDiskData {
        pub(super) fn failure(&self) -> Option<&ChunkDiskFailure> {
            self.failure.as_ref()
        }
        pub(super) fn attribute_failure(&mut self, error: ChunkDiskFailure) {
            self.failure = Some(error);
        }
    }
    pub(super) mod chunk_disk {
        pub(crate) fn unwound_failure(mod_id: &'static str) -> super::ChunkDiskFailure {
            super::ChunkDiskFailure {
                mod_id: mod_id.into(),
                reason: "native listener unwound",
            }
        }
    }
}
use stubs::chunk_disk;

impl ChunkDiskHost for TestHost {}

#[test]
fn a_chunk_job_continues_after_panic_and_codec_failure() {
    #[derive(Clone, Copy)]
    enum Write {
        Panic,
        Accept(i32),
        FailCodec,
    }

    struct DiskListener {
        expected: i32,
        write: Write,
    }
    #[mod_api::__subscriptions]
    impl Hooks for DiskListener {
        fn chunk_data_load(&self, _: &dyn ChunkDiskHost, event: &mut ChunkDataLoadEvent<'_>) {
            assert_eq!(event.data.value, self.expected);
            assert!(event.data.failure().is_none());
            match self.write {
                Write::Panic => {
                    event.data.value = 99;
                    panic!("tentative chunk write");
                }
                Write::Accept(value) => event.data.value = value,
                Write::FailCodec => {
                    event.data.value = 99;
                    event.data.failure = Some(ChunkDiskFailure {
                        reason: "attachment codec refused",
                        ..ChunkDiskFailure::default()
                    });
                }
            }
        }
    }

    install(
        [
            ("disk_panic", 7, Write::Panic),
            ("disk_first", 7, Write::Accept(8)),
            ("disk_codec", 8, Write::FailCodec),
            ("disk_later", 8, Write::Accept(9)),
        ]
        .into_iter()
        .map(|(id, expected, write)| ModEntry::new(id, Box::new(DiskListener { expected, write })))
        .collect(),
    );
    let mut job = ChunkDiskData {
        value: 7,
        ..ChunkDiskData::default()
    };
    mod_api::dispatch_chunk_data_load(&TestHost, &mut job);
    assert_eq!(job.value, 9);
    let failure = job.failure().unwrap();
    assert_eq!(failure.mod_id, "disk_panic");
    assert_eq!(failure.reason, "native listener unwound");
    assert!(runtime::poisoned(HookId::ChunkDataLoad, "disk_panic"));
    assert!(!runtime::poisoned(HookId::ChunkDataLoad, "disk_codec"));
}
