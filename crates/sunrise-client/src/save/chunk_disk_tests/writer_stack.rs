use super::*;
use std::sync::OnceLock;
use std::thread::{self, ThreadId};

#[derive(Debug)]
struct WriterThread {
    id: ThreadId,
    #[cfg(target_os = "linux")]
    stack_bytes: Option<usize>,
}

#[derive(Default)]
struct StackProbe {
    writer: Arc<OnceLock<WriterThread>>,
}

#[mod_api::sunrise_mod]
impl Hooks for StackProbe {
    fn chunk_data_save(&self, _: &dyn ChunkDiskHost, _: &mut ChunkDataSaveEvent<'_>) {
        self.writer
            .set(WriterThread {
                id: thread::current().id(),
                #[cfg(target_os = "linux")]
                stack_bytes: sunrise_threads::unwind::StackBounds::current()
                    .map(|bounds| bounds.hi - bounds.lo),
            })
            .expect("the save hook ran more than once");
    }
}

#[test]
#[ignore = "daily"]
fn asynchronous_chunk_save_hook_runs_on_the_region_writer() {
    // Subscribed saves consult a process-wide lease. A parallel test must not block this save.
    const CHILD: &str = "SUNRISE_REGION_WRITER_STACK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "save::chunk_disk_tests::writer_stack::asynchronous_chunk_save_hook_runs_on_the_region_writer",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let caller = thread::current().id();
    let probe = StackProbe::default();
    let writer = probe.writer.clone();
    let mods = vec![mod_api::ModEntry::new("stack_probe", Box::new(probe))].into();
    assert_eq!(
        save_chunk(mods),
        7,
        "the queued chunk reached the region file"
    );
    let writer = writer.get().expect("the save hook did not run");
    assert_ne!(writer.id, caller, "the save hook ran on the calling thread");
    #[cfg(target_os = "linux")]
    {
        let minimum = sunrise_threads::GAME_THREAD_STACK_BYTES;
        let bytes = writer.stack_bytes.expect("could not read the writer stack");
        assert!(
            bytes >= minimum,
            "the writer stack was {bytes} bytes, below {minimum}"
        );
    }
}

fn save_chunk(mods: mod_api::Mods) -> i32 {
    let (registry, biomes) = super::super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = tempfile::tempdir().unwrap();
    mod_platform::testing::with_mods(mods, || {
        let save = SaveDir::new(
            path.path().to_path_buf(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        save.save_later(&[Arc::new(chunk(7))], registry, &biomes)
            .unwrap();
        save.flush().unwrap();
    });
    disk_value(path.path(), registry, &biomes).unwrap()
}
