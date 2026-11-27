//! Starling: World files written off the render thread while the game saves in the background (the
//! autosave): Android's shared storage takes milliseconds per file call, and a save writes a
//! dozen small files. One thread writes them in order; reads see what is still queued; a
//! synchronous write, and [`flush`], wait for the queue first.
use super::{Path, PathBuf, Tag};
use std::{
    cell::Cell,
    collections::HashMap,
    sync::{Condvar, Mutex, OnceLock, mpsc::Sender},
};

pub enum Job {
    Dat(Tag),
    Bytes(Vec<u8>),
}
impl Job {
    /// The bytes the file will hold (what a read must see before the write is done).
    fn bytes(&self) -> Option<Vec<u8>> {
        match self {
            Job::Bytes(b) => Some(b.clone()),
            Job::Dat(tag) => {
                let named = sunrise_core::nbt::NamedTag {
                    name: "".into(),
                    tag: tag.clone(),
                };
                sunrise_core::nbt::write_named(&named, sunrise_core::nbt::Limits::default()).ok()
            }
        }
    }
}
struct Queue {
    tx: Mutex<Option<Sender<(u64, PathBuf, Job)>>>,
    /// The newest queued bytes of each file, by the job that will write them.
    pending: Mutex<HashMap<PathBuf, (u64, Vec<u8>)>>,
    /// (jobs queued, jobs written).
    counts: Mutex<(u64, u64)>,
    done: Condvar,
}
thread_local! {
    static DEFERRED: Cell<bool> = const { Cell::new(false) };
}
fn queue_state() -> &'static Queue {
    static QUEUE: OnceLock<Queue> = OnceLock::new();
    QUEUE.get_or_init(|| Queue {
        tx: Mutex::new(None),
        pending: Mutex::default(),
        counts: Mutex::new((0, 0)),
        done: Condvar::new(),
    })
}
/// Runs `f` with its world file writes queued.
pub fn deferred<R>(f: impl FnOnce() -> R) -> R {
    DEFERRED.with(|d| d.set(true));
    let out = f();
    DEFERRED.with(|d| d.set(false));
    out
}
pub(super) fn is_deferred() -> bool {
    DEFERRED.with(Cell::get)
}
pub(super) fn queue(path: &Path, job: Job) {
    let q = queue_state();
    let mut counts = q.counts.lock().unwrap_or_else(|e| e.into_inner());
    let seq = counts.0 + 1;
    if let Some(bytes) = job.bytes()
        && let Ok(mut pending) = q.pending.lock()
    {
        pending.insert(path.to_owned(), (seq, bytes));
    }
    let mut tx = q.tx.lock().unwrap_or_else(|e| e.into_inner());
    if tx.is_none() {
        let (send, receive) = std::sync::mpsc::channel::<(u64, PathBuf, Job)>();
        let started = std::thread::Builder::new()
            .name("sunrise-world-files".into())
            .spawn(move || {
                for (seq, path, job) in receive {
                    let written = match job {
                        Job::Dat(tag) => super::level_dat::write_dat_now(&path, tag),
                        Job::Bytes(bytes) => super::write_file_now(&path, &bytes),
                    };
                    if let Err(e) = written {
                        log::warn!("Sunrise: could not save {}: {e}", path.display());
                    }
                    let q = queue_state();
                    if let Ok(mut pending) = q.pending.lock()
                        && pending.get(&path).is_some_and(|(s, _)| *s == seq)
                    {
                        pending.remove(&path);
                    }
                    let mut counts = q.counts.lock().unwrap_or_else(|e| e.into_inner());
                    counts.1 += 1;
                    q.done.notify_all();
                }
            });
        if started.is_err() {
            drop(tx);
            drop(counts);
            // No writer thread: write it now.
            let _ = match job {
                Job::Dat(tag) => super::level_dat::write_dat_now(path, tag),
                Job::Bytes(bytes) => super::write_file_now(path, &bytes),
            };
            if let Ok(mut pending) = q.pending.lock() {
                pending.remove(path);
            }
            return;
        }
        *tx = Some(send);
    }
    counts.0 = seq;
    if let Some(send) = tx.as_ref() {
        let _ = send.send((seq, path.to_owned(), job));
    }
}
/// The newest queued bytes of `path`, when its write is not done yet.
pub(super) fn queued(path: &Path) -> Option<Vec<u8>> {
    let q = queue_state();
    q.pending
        .lock()
        .ok()?
        .get(path)
        .map(|(_, bytes)| bytes.clone())
}
/// Waits until every queued file is written.
pub fn flush() {
    let q = queue_state();
    let mut counts = q.counts.lock().unwrap_or_else(|e| e.into_inner());
    while counts.1 < counts.0 {
        counts = q.done.wait(counts).unwrap_or_else(|e| e.into_inner());
    }
}
