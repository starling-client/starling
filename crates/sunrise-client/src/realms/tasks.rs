// Included in realms.rs to retain its private scope.
/// What a worker thread hands back.
type Job<T> = Arc<Mutex<Option<T>>>;

fn spawn<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Job<T> {
    let job = Arc::new(Mutex::new(None));
    let out = Arc::clone(&job);
    let started = thread::Builder::new().name("Realms".into()).spawn(move || {
        let result = work();
        if let Ok(mut slot) = out.lock() {
            *slot = Some(result);
        }
    });
    if let Err(e) = started {
        log::warn!("Sunrise: could not start a Realms worker: {e}");
    }
    job
}

fn take<T>(job: &Job<T>) -> Option<T> {
    job.lock().ok()?.take()
}

/// `RealmsLongRunningMcoTaskScreen`'s running task: the worker's job, the title the
/// screen shows while it runs (`LongRunningTask.getTitle`, updated as the next task
/// starts), and the flag `cancel()` sets (`LongRunningTask.abortTask`, checked
/// between the steps).
#[derive(Clone, Debug)]
pub struct RunningTask {
    job: Job<Result<(), RealmsError>>,
    title: Arc<Mutex<String>>,
    abort: Arc<AtomicBool>,
}

/// `DownloadTask`: the download-info request on a worker (`DownloadTask.run`'s retry loop)
/// and the flag its cancel sets (`LongRunningTask.aborted`).
#[derive(Clone, Debug)]
pub struct DownloadTask {
    job: Job<Result<WorldDownload, RealmsError>>,
    abort: Arc<AtomicBool>,
}

/// `RestoreTask`: `RealmsClient.restoreWorld` on a worker, retried while the service says
/// to wait, and the flag its cancel sets (`LongRunningTask.aborted`).
#[derive(Clone, Debug)]
pub struct RestoreTask {
    job: Job<Result<(), RealmsError>>,
    abort: Arc<AtomicBool>,
}
