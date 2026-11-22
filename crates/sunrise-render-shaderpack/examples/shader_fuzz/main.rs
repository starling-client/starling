//! The Rust side of tools/shader_fuzz.py: a seeded robustness fuzzer of the shader-pack
//! translator. Each case is a pack (the committed seed pack, or a pack of `--corpus`) cut down
//! to one or two programs, mutated by a few named steps, and loaded through `load_post` as the
//! game loads a pack, with the game's own inputs (`pack_atlas::game_inputs`).
//!
//!     shader_fuzz [--seed N] [--count N] [--budget SECONDS] [--corpus DIR] [--jobs N]
//!                 [--replay SEED] [--smoke] [--time-limit SECONDS] [--memory-limit MIB]
//!                 [--size-limit MIB] [--shrink-seconds N] [--work-dir DIR]
//!
//! Every case keeps these invariants: no panic; done within `--time-limit`; within
//! `--memory-limit` (the worker's allocator refuses past it, which aborts it); every stage file
//! within `--size-limit` once preprocessed; every program that loads validates in naga; every
//! note names its subject, and a program the unmutated case loaded is loaded or named; the same
//! case loads the same twice. Cases run in worker processes (this binary, `worker`), so a hang
//! or an abort costs one case. A failure is grouped by invariant, first failing step and
//! detail; each group's smallest case is shrunk while the same failure stays, written as a pack
//! folder under `<work-dir>/groups/`, and listed in `<work-dir>/report.json` with the seed that
//! replays it. Exits 1 when any case fails.

mod cases;
mod check;
mod shrink;

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use cases::{Case, Source};
use check::{Failure, Outcome, Request};
use shrink::{Budget, Evaluate, Files, Found};

/// The system allocator, counting: past `LIMIT` bytes in use an allocation fails, and the
/// process aborts with Rust's "memory allocation of N bytes failed".
struct Counting;

static USED: AtomicUsize = AtomicUsize::new(0);
static LIMIT: AtomicUsize = AtomicUsize::new(usize::MAX);

impl Counting {
    fn take(size: usize) -> bool {
        let before = USED.fetch_add(size, Ordering::Relaxed);
        if before.saturating_add(size) > LIMIT.load(Ordering::Relaxed) {
            USED.fetch_sub(size, Ordering::Relaxed);
            return false;
        }
        true
    }
}

// SAFETY: every call forwards to `System` with the caller's layout; the count only decides
// whether to forward, and a refused allocation returns null as `GlobalAlloc` allows.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !Self::take(layout.size()) {
            return std::ptr::null_mut();
        }
        // SAFETY: the caller's layout, as `GlobalAlloc::alloc` requires.
        let p = unsafe { System.alloc(layout) };
        if p.is_null() {
            USED.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !Self::take(layout.size()) {
            return std::ptr::null_mut();
        }
        // SAFETY: as `alloc`.
        let p = unsafe { System.alloc_zeroed(layout) };
        if p.is_null() {
            USED.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        USED.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: `ptr` came from this allocator with `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let grow = new_size.saturating_sub(layout.size());
        if !Self::take(grow) {
            return std::ptr::null_mut();
        }
        // SAFETY: `ptr` came from this allocator with `layout`.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if p.is_null() {
            USED.fetch_sub(grow, Ordering::Relaxed);
        } else {
            USED.fetch_sub(layout.size().saturating_sub(new_size), Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

struct Options {
    seed: u64,
    count: u64,
    budget: Option<Duration>,
    corpus: Option<PathBuf>,
    jobs: usize,
    replay: Option<u64>,
    smoke: bool,
    time_limit: Duration,
    memory_mib: usize,
    size_limit: usize,
    shrink: Duration,
    work: PathBuf,
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        seed: 1,
        count: 200,
        budget: None,
        corpus: None,
        jobs: 1,
        replay: None,
        smoke: false,
        time_limit: Duration::from_secs(20),
        memory_mib: 2048,
        size_limit: 16 << 20,
        shrink: Duration::from_secs(120),
        work: PathBuf::from(".local/shader-fuzz"),
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value")).cloned();
        let number = |v: String| v.parse::<u64>().map_err(|_| format!("{arg}: not a number"));
        match arg.as_str() {
            "--seed" => o.seed = number(value()?)?,
            "--count" => o.count = number(value()?)?,
            "--budget" => o.budget = Some(Duration::from_secs(number(value()?)?)),
            "--corpus" => o.corpus = Some(PathBuf::from(value()?)),
            "--jobs" => o.jobs = number(value()?)?.max(1) as usize,
            "--replay" => o.replay = Some(number(value()?)?),
            "--smoke" => o.smoke = true,
            "--time-limit" => o.time_limit = Duration::from_secs(number(value()?)?),
            "--memory-limit" => o.memory_mib = number(value()?)? as usize,
            "--size-limit" => o.size_limit = (number(value()?)? as usize) << 20,
            "--shrink-seconds" => o.shrink = Duration::from_secs(number(value()?)?),
            "--work-dir" => o.work = PathBuf::from(value()?),
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    Ok(o)
}

// ---------------------------------------------------------------------------------------------
// Running cases

/// Explicit destinations may be new or empty. Claim an empty folder before any worker
/// runs, so two processes cannot both observe it empty and start deleting scratch files.
fn claim_folder(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    if std::fs::read_dir(dir)?.next().transpose()?.is_some() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "work directory is not empty",
        ));
    }
    std::fs::File::create_new(dir.join(".shader-fuzz-run"))?;
    Ok(())
}

fn default_folder(base: &Path) -> std::io::Result<PathBuf> {
    let runs = base.join("runs");
    std::fs::create_dir_all(&runs)?;
    for index in 0u64.. {
        let dir = runs.join(format!("{}-{index}", std::process::id()));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

/// Only ordinary path components can be created below the run's shader directory.
fn safe_component(part: &str) -> bool {
    if part.is_empty()
        || part.ends_with(['.', ' '])
        || part.contains(['\\', ':', '<', '>', '"', '|', '?', '*'])
        || part.chars().any(char::is_control)
    {
        return false;
    }
    let stem = part
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let device = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    );
    let port = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"));
    !device
        && !port.is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
}

/// Validate the whole pack before replacing any previous scratch data.
fn write_pack(dir: &Path, files: &Files) -> std::io::Result<()> {
    if let Some(path) = files.keys().find(|p| !p.split('/').all(safe_component)) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid pack path {path:?}"),
        ));
    }
    let root = dir.join("shaders");
    match std::fs::remove_dir_all(&root) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::fs::create_dir_all(&root)?;
    for (path, bytes) in files {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap())?;
        std::fs::File::create_new(file)?.write_all(bytes)?;
    }
    Ok(())
}

/// An I/O error belongs to the driver; it must never be recorded as a translator finding.
fn output<T>(path: &Path, result: std::io::Result<T>) -> T {
    result.unwrap_or_else(|e| {
        eprintln!("shader_fuzz: staging {}: {e}", path.display());
        std::process::exit(2)
    })
}

/// Runs cases in this process (tests; the time and memory bounds don't apply).
#[cfg(test)]
struct InProcess {
    dir: PathBuf,
}

#[cfg(test)]
impl Evaluate for InProcess {
    fn eval(&mut self, files: &Files, request: &Request) -> Outcome {
        write_pack(&self.dir, files).expect("test pack is writable");
        check::check(&self.dir, request)
    }
}

fn request_json(dir: &Path, r: &Request) -> String {
    serde_json::json!({
        "dir": dir, "dim": r.dim, "flat": r.flat, "expect": r.expect, "size_limit": r.size_limit,
    })
    .to_string()
}

fn outcome_json(o: &Outcome) -> String {
    let failures: Vec<_> = o
        .failures
        .iter()
        .map(|f| [&f.invariant, &f.detail])
        .collect();
    serde_json::json!({"failures": failures, "loaded": o.loaded, "shadow_pass": o.shadow_pass})
        .to_string()
}

fn parse_outcome(line: &str) -> Option<Outcome> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let strings = |v: &serde_json::Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect()
    };
    Some(Outcome {
        failures: v["failures"]
            .as_array()?
            .iter()
            .map(|f| {
                let pair = strings(f);
                Failure::new(&pair[0], pair[1].clone())
            })
            .collect(),
        loaded: strings(&v["loaded"]),
        shadow_pass: v["shadow_pass"].as_bool()?,
    })
}

/// The worker: requests in, outcomes out, a line each.
fn worker(memory_mib: usize) {
    std::panic::set_hook(Box::new(|info| {
        let at = info
            .location()
            .map_or(String::new(), |l| format!("{}:{}", l.file(), l.line()));
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let text = format!("{at}: {}", check::shape(&message));
        check::PANIC_AT.with(|p| *p.borrow_mut() = Some(text));
    }));
    LIMIT.store(memory_mib << 20, Ordering::Relaxed);
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let v: serde_json::Value = serde_json::from_str(&line).expect("a request");
        let request = Request {
            dim: v["dim"].as_u64().unwrap_or(0) as usize,
            flat: v["flat"].as_bool().unwrap_or(true),
            expect: v["expect"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect(),
            size_limit: v["size_limit"].as_u64().unwrap_or(u64::MAX) as usize,
        };
        let dir = PathBuf::from(v["dir"].as_str().unwrap_or("."));
        let outcome = check::check(&dir, &request);
        let _ = writeln!(out, "{}", outcome_json(&outcome));
        let _ = out.flush();
    }
}

/// A worker process, restarted after a case it could not finish.
struct Worker {
    dir: PathBuf,
    memory_mib: usize,
    time_limit: Duration,
    running: Option<Running>,
}

struct Running {
    child: std::process::Child,
    input: std::process::ChildStdin,
    lines: Receiver<String>,
    errors: std::thread::JoinHandle<Vec<u8>>,
}

impl Worker {
    fn start(&mut self) -> &mut Running {
        self.running.get_or_insert_with(|| {
            let exe = std::env::current_exe().expect("this binary");
            let mut child = Command::new(exe)
                .args(["worker", &self.memory_mib.to_string()])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("a worker starts");
            let input = child.stdin.take().expect("piped");
            let output = child.stdout.take().expect("piped");
            let mut errors_in = child.stderr.take().expect("piped");
            let (tx, lines) = channel();
            std::thread::spawn(move || {
                for line in BufReader::new(output).lines().map_while(Result::ok) {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            });
            // The last 8 KiB the worker wrote to stderr, once it has exited (end of the pipe).
            let errors = std::thread::spawn(move || {
                let (mut tail, mut buf) = (Vec::new(), [0u8; 4096]);
                while let Ok(n) = errors_in.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    tail.extend_from_slice(&buf[..n]);
                    let excess = tail.len().saturating_sub(8192);
                    tail.drain(..excess);
                }
                tail
            });
            Running {
                child,
                input,
                lines,
                errors,
            }
        })
    }
}

impl Evaluate for Worker {
    fn eval(&mut self, files: &Files, request: &Request) -> Outcome {
        let dir = self.dir.clone();
        output(&dir, write_pack(&dir, files));
        let limit = self.time_limit;
        let running = self.start();
        let sent = writeln!(running.input, "{}", request_json(&dir, request))
            .and_then(|_| running.input.flush());
        let reply = match sent {
            Ok(()) => running.lines.recv_timeout(limit),
            Err(_) => Err(RecvTimeoutError::Disconnected),
        };
        if let Ok(line) = &reply
            && let Some(outcome) = parse_outcome(line)
        {
            return outcome;
        }
        let mut gone = self.running.take().expect("running");
        let _ = gone.child.kill();
        let status = gone.child.wait();
        // All the worker wrote: the reader ends at the end of the pipe, which the exit closed.
        let errors = gone.errors.join().unwrap_or_default();
        let errors = String::from_utf8_lossy(&errors).into_owned();
        let failure = match reply {
            // Only this depends on timing: a case that takes about the limit can land either side.
            Err(RecvTimeoutError::Timeout) => {
                Failure::new("time", format!("over {} s", limit.as_secs()))
            }
            _ if errors.contains("memory allocation of") => {
                Failure::new("memory", format!("over {} MiB", self.memory_mib))
            }
            _ => Failure::new(
                "crash",
                format!(
                    "{}: {}",
                    status.map_or("?".into(), |s| s.to_string()),
                    check::shape(errors.lines().last().unwrap_or(""))
                ),
            ),
        };
        Outcome {
            failures: vec![failure],
            ..Default::default()
        }
    }
}

/// The programs that must load or be named in `case` with `files`: those the unmutated case
/// loaded whose own stage files are the only ones changed. A `program.` key in the properties
/// can turn a program off without a note, by design: then none.
fn expect(case: &Case, base: &Outcome, files: &Files) -> Vec<String> {
    let properties = case
        .files
        .get("shaders.properties")
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default();
    if properties.contains("program.") {
        return Vec::new();
    }
    let touched: Vec<&String> = case
        .files
        .keys()
        .filter(|p| files.get(*p) != case.files.get(*p))
        .collect();
    base.loaded
        .iter()
        .filter(|name| touched.iter().all(|t| case.owns(name, t)))
        .cloned()
        .collect()
}

type BaseKey = (String, usize, bool, Vec<String>);

/// What every worker thread shares.
struct Run<'a> {
    corpus: &'a [Source],
    o: &'a Options,
    next: AtomicU64,
    started: Instant,
    bases: Mutex<HashMap<BaseKey, Outcome>>,
    found: Mutex<Vec<Found>>,
    done: AtomicU64,
}

impl Run<'_> {
    fn request(&self, case: &Case, expect: Vec<String>) -> Request {
        Request {
            dim: case.dim,
            flat: case.flat,
            expect,
            size_limit: self.o.size_limit,
        }
    }

    /// Case `seed` through `ev`: its failures, each with its first failing step.
    fn run_case(&self, ev: &mut dyn Evaluate, case: &Case) {
        let key = (
            case.pack.clone(),
            case.dim,
            case.flat,
            case.programs.clone(),
        );
        let cached = self.bases.lock().unwrap().get(&key).cloned();
        let base = cached.unwrap_or_else(|| {
            let base = ev.eval(&case.files, &self.request(case, Vec::new()));
            self.bases.lock().unwrap().insert(key, base.clone());
            base
        });
        let files = case.materialize();
        let request = self.request(case, expect(case, &base, &files));
        let outcome = ev.eval(&files, &request);
        for failure in outcome.failures {
            let step = shrink::first_step(ev, case, &request, &failure.signature());
            self.found.lock().unwrap().push(Found {
                case: case.clone(),
                request: request.clone(),
                failure,
                step,
            });
        }
        let done = self.done.fetch_add(1, Ordering::Relaxed) + 1;
        if done.is_multiple_of(100) {
            println!(
                "{done} cases, {} failures, {:.0} s",
                self.found.lock().unwrap().len(),
                self.started.elapsed().as_secs_f64()
            );
        }
    }

    fn cases(&self, ev: &mut dyn Evaluate, seeds: &[Case]) {
        loop {
            let i = self.next.fetch_add(1, Ordering::Relaxed);
            let case = match seeds.get(i as usize) {
                Some(case) => case.clone(),
                None if !seeds.is_empty() => break,
                None => {
                    let over = self.o.budget.is_some_and(|b| self.started.elapsed() >= b);
                    if i >= self.o.count || over {
                        break;
                    }
                    let seed = cases::case_seed(self.o.seed, i);
                    match cases::generate(self.corpus, seed, true) {
                        Some(case) => case,
                        None => continue,
                    }
                }
            };
            self.run_case(ev, &case);
        }
    }
}

/// Fixed cases that must hold every invariant: each seed program alone, unmutated, in flat and
/// grid terrain, and all of them together.
fn smoke(corpus: &[Source]) -> Vec<Case> {
    let seeds = &corpus[0];
    let (folder, programs) = &seeds.folders[0];
    let mut sets: Vec<Vec<String>> = programs.iter().map(|p| vec![p.clone()]).collect();
    sets.push(programs.clone());
    let mut out = Vec::new();
    for flat in [true, false] {
        for set in &sets {
            out.push(Case {
                seed: 0,
                pack: seeds.name.clone(),
                dim: 0,
                flat,
                programs: set.clone(),
                folder: folder.clone(),
                files: cases::subset(seeds, folder, set),
                steps: Vec::new(),
            });
        }
    }
    out
}

/// A group's report entry, its shrunk case written as a pack under `dir`.
fn write_group(
    dir: &Path,
    name: &str,
    members: &[Found],
    shrunk: &shrink::Shrunk,
) -> serde_json::Value {
    let first = &members[0];
    output(dir, write_pack(dir, &shrunk.files));
    let steps: Vec<String> = shrunk
        .steps
        .iter()
        .map(|s| format!("{} {}", s.kind.name(), s.file))
        .collect();
    let lines = shrunk
        .files
        .values()
        .map(|b| b.split(|&c| c == b'\n').filter(|l| !l.is_empty()).count())
        .sum::<usize>();
    let summary = format!(
        "group: {name}\nreplay: --replay {}\npack: {}\nprograms: {}\ndimension: {}\nterrain: {}\nsteps: {}\n",
        first.case.seed,
        first.case.pack,
        first.case.programs.join(" "),
        cases::DIMENSIONS[first.case.dim].0,
        if first.case.flat { "flat" } else { "grid" },
        steps.join(", "),
    );
    let summary_path = dir.join("case.txt");
    output(&summary_path, std::fs::write(&summary_path, summary));
    serde_json::json!({
        "group": name,
        "invariant": first.failure.invariant,
        "step": first.step,
        "detail": first.failure.detail,
        "count": members.len(),
        "seeds": members.iter().take(20).map(|m| m.case.seed).collect::<Vec<_>>(),
        "replay": first.case.seed,
        "pack": first.case.pack,
        "programs": first.case.programs,
        "dimension": cases::DIMENSIONS[first.case.dim].0,
        "flat": first.case.flat,
        "steps": steps,
        "files": shrunk.files.keys().collect::<Vec<_>>(),
        "lines": lines,
        "dir": dir,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("worker") {
        let mib = args.get(1).and_then(|m| m.parse().ok()).unwrap_or(2048);
        return worker(mib);
    }
    let mut o = match options(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("shader_fuzz: {e}");
            std::process::exit(2);
        }
    };
    if !args.iter().any(|arg| arg == "--work-dir") {
        o.work = output(&o.work, default_folder(&o.work));
    }
    output(&o.work, claim_folder(&o.work));
    let corpus = cases::corpus(o.corpus.as_deref());
    let names: Vec<&str> = corpus.iter().map(|s| s.name.as_str()).collect();
    println!("corpus: {}", names.join(", "));
    let fixed: Vec<Case> = if o.smoke {
        smoke(&corpus)
    } else if let Some(seed) = o.replay {
        vec![cases::generate(&corpus, seed, true).expect("the seed makes a case with this corpus")]
    } else {
        Vec::new()
    };
    let run = Run {
        corpus: &corpus,
        o: &o,
        next: AtomicU64::new(0),
        started: Instant::now(),
        bases: Mutex::new(HashMap::new()),
        found: Mutex::new(Vec::new()),
        done: AtomicU64::new(0),
    };
    let worker = |n: usize| Worker {
        dir: o.work.join(format!("worker{n}")),
        memory_mib: o.memory_mib,
        time_limit: o.time_limit,
        running: None,
    };
    std::thread::scope(|scope| {
        for n in 0..o.jobs {
            let (run, fixed, mut ev) = (&run, &fixed, worker(n));
            scope.spawn(move || run.cases(&mut ev, fixed));
        }
    });
    let elapsed = run.started.elapsed();
    let found = run.found.into_inner().unwrap();
    let failing = found.len();
    let groups: Vec<(String, Vec<Found>)> = shrink::group(found).into_iter().collect();
    // Each group's smallest case, shrunk; the groups share the workers.
    let next = AtomicUsize::new(0);
    let entries: Mutex<BTreeMap<usize, serde_json::Value>> = Mutex::new(BTreeMap::new());
    std::thread::scope(|scope| {
        for n in 0..o.jobs.min(groups.len()) {
            let (groups, next, entries, mut ev, o) = (&groups, &next, &entries, worker(n), &o);
            scope.spawn(move || {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((name, members)) = groups.get(i) else {
                        break;
                    };
                    let first = &members[0];
                    let budget = Budget {
                        evals: 4000,
                        until: Instant::now() + o.shrink,
                    };
                    let shrunk = shrink::shrink(
                        &mut ev,
                        &first.case,
                        &first.request,
                        &first.failure.signature(),
                        budget,
                    );
                    let group = format!("{:02}-{}", i + 1, first.failure.invariant);
                    let base = o.work.join("groups");
                    if !safe_component(&group) || group.contains('/') {
                        output(
                            &base,
                            Err::<(), _>(std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                "invalid group directory name",
                            )),
                        );
                    }
                    let dir = base.join(group);
                    let entry = write_group(&dir, name, members, &shrunk);
                    entries.lock().unwrap().insert(i, entry);
                }
            });
        }
    });
    let entries: Vec<serde_json::Value> = entries.into_inner().unwrap().into_values().collect();
    let report = serde_json::json!({
        "seed": o.seed,
        "cases": run.done.load(Ordering::Relaxed),
        "failing": failing,
        "seconds": elapsed.as_secs(),
        "corpus": names,
        "corpus_dir": o.corpus,
        "groups": entries,
    });
    let path = o.work.join("report.json");
    output(
        &path,
        std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap() + "\n"),
    );
    println!(
        "{} cases in {:.0} s: {failing} failures in {} groups",
        report["cases"],
        elapsed.as_secs_f64(),
        entries.len()
    );
    for g in &entries {
        println!(
            "  {:4}  {}  (replay: --replay {}, {} lines in {})",
            g["count"],
            g["group"].as_str().unwrap_or(""),
            g["replay"],
            g["lines"],
            g["dir"].as_str().unwrap_or("")
        );
    }
    println!("report: {}", path.display());
    if !entries.is_empty() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    fn temp() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "shader-fuzz-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn storage_preserves_existing_results_and_claims_a_directory_once() {
        let dir = temp();
        let previous = dir.join("groups");
        std::fs::create_dir_all(&previous).unwrap();
        std::fs::write(previous.join("keep"), b"previous").unwrap();
        assert!(claim_folder(&dir).is_err());
        let first = default_folder(&dir).unwrap();
        let second = default_folder(&dir).unwrap();
        assert_ne!(first, second);
        std::thread::scope(|scope| {
            let a = scope.spawn(|| claim_folder(&first).is_ok());
            let b = scope.spawn(|| claim_folder(&first).is_ok());
            assert_ne!(a.join().unwrap(), b.join().unwrap());
        });
        assert_eq!(std::fs::read(previous.join("keep")).unwrap(), b"previous");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn storage_rejects_nonportable_paths_before_writing() {
        let dir = temp();
        let files = Files::from([("ok.glsl".into(), b"previous".to_vec())]);
        write_pack(&dir, &files).unwrap();
        for name in [
            "../x",
            "C:/x",
            "a\\b",
            "/x",
            "./x",
            "NUL.txt",
            "sub/abc.",
            "COM1",
            "LPT².txt",
            "file:stream",
        ] {
            let files = Files::from([(name.into(), Vec::new())]);
            assert!(write_pack(&dir, &files).is_err(), "{name}");
        }
        assert_eq!(
            std::fs::read(dir.join("shaders/ok.glsl")).unwrap(),
            b"previous"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn opts(work: PathBuf) -> Options {
        let mut o = options(&[]).unwrap();
        o.work = work;
        o
    }

    fn run_with<'a>(corpus: &'a [Source], o: &'a Options) -> Run<'a> {
        Run {
            corpus,
            o,
            next: AtomicU64::new(0),
            started: Instant::now(),
            bases: Mutex::new(HashMap::new()),
            found: Mutex::new(Vec::new()),
            done: AtomicU64::new(0),
        }
    }

    #[test]
    fn the_seed_pack_holds_every_invariant() {
        let corpus = cases::corpus(None);
        let dir = temp();
        let mut ev = InProcess { dir: dir.clone() };
        let all = smoke(&corpus);
        let together = all.iter().find(|c| c.programs.len() == 7).unwrap();
        let outcome = ev.eval(
            &together.files,
            &Request {
                dim: 0,
                flat: true,
                expect: together.programs.clone(),
                size_limit: 16 << 20,
            },
        );
        assert_eq!(outcome.failures, []);
        // The passes in their order (`composite1` of compute shaders only), then the geometry
        // slots: terrain, water, the shadow pass
        // (on: `deferred` reads `shadowtex0`).
        assert_eq!(
            outcome.loaded,
            [
                "deferred",
                "composite",
                "composite1",
                "final",
                "gbuffers_terrain",
                "gbuffers_water",
                "shadow",
            ]
            .map(str::to_owned)
        );
        assert!(outcome.shadow_pass);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fixed_seeds_hold_every_invariant() {
        let corpus = cases::corpus(None);
        let dir = temp();
        let mut o = opts(dir.clone());
        // Run seed 2: cases of every kind of step that hold, quick enough to run in-process
        // (without the worker's time and memory bounds).
        o.seed = 2;
        o.count = 4;
        let run = run_with(&corpus, &o);
        let mut ev = InProcess { dir: dir.join("w") };
        run.cases(&mut ev, &[]);
        assert_eq!(run.done.load(Ordering::Relaxed), 4);
        let found = run.found.into_inner().unwrap();
        assert!(
            found.is_empty(),
            "{:?}",
            found.iter().map(Found::group).collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A fake translator that panics on the anonymous struct the `declaration` step writes, fails
    /// validation on its buffer block, and drops `composite` without a note when one of its
    /// stage files names anything a step wrote (`fz_`).
    fn planted(files: &Files, request: &Request) -> Outcome {
        let text: String = files.values().map(|b| String::from_utf8_lossy(b)).collect();
        let mut failures = Vec::new();
        if text.contains("struct { float x; vec2 y[3]; } fz_anon, fz_anon2;") {
            failures.push(Failure::new("panic", "planted.rs:1: anonymous"));
        }
        if text.contains("buffer FzBuf { uint fz_count; vec4 fz_data[]; } fz_buf;") {
            failures.push(Failure::new("naga", "composite: planted"));
        }
        let stages = [files.get("composite.vsh"), files.get("composite.fsh")];
        let loads = stages
            .iter()
            .all(|f| f.is_some_and(|b| !String::from_utf8_lossy(b).contains("fz_")));
        let loaded: Vec<String> = loads.then(|| "composite".to_owned()).into_iter().collect();
        if request.expect.iter().any(|p| p == "composite") && !loads {
            failures.push(Failure::new("refusal", "composite: not loaded, no note"));
        }
        Outcome {
            failures,
            loaded,
            ..Default::default()
        }
    }

    fn shrunk(ev: &mut dyn Evaluate, first: &Found) -> shrink::Shrunk {
        let budget = Budget {
            evals: 10_000,
            until: Instant::now() + Duration::from_secs(60),
        };
        let signature = first.failure.signature();
        shrink::shrink(ev, &first.case, &first.request, &signature, budget)
    }

    #[test]
    fn a_planted_failure_is_shrunk_and_grouped() {
        let corpus = cases::corpus(None);
        let mut o = opts(temp());
        o.count = 1000;
        let run = run_with(&corpus, &o);
        let mut ev = planted;
        run.cases(&mut ev, &[]);
        let groups = shrink::group(run.found.into_inner().unwrap());
        let (refusals, others): (Vec<_>, Vec<_>) = groups
            .iter()
            .partition(|(name, _)| name.starts_with("refusal|"));
        assert_eq!(
            others.iter().map(|g| g.0).collect::<Vec<_>>(),
            [
                "naga|declaration|composite: planted",
                "panic|declaration|planted.rs:1: anonymous"
            ]
        );
        for (name, members) in others {
            assert!(members.len() > 1, "{name}");
            let first = &members[0];
            let shrunk = shrunk(&mut ev, first);
            // One step, one file, one line: the planted declaration.
            assert_eq!(shrunk.steps.len(), 1, "{name}");
            assert_eq!(shrunk.steps[0].kind, cases::Kind::Declaration);
            assert_eq!(shrunk.files.len(), 1, "{name}");
            let text = String::from_utf8(shrunk.files.values().next().unwrap().clone()).unwrap();
            let line = if name.starts_with("panic") {
                "struct { float x; vec2 y[3]; } fz_anon, fz_anon2;"
            } else {
                "layout(std430, binding = 3) buffer FzBuf { uint fz_count; vec4 fz_data[]; } fz_buf;"
            };
            assert_eq!(text, line);
            // The replay seed makes the same case, which fails the same way.
            let again = cases::generate(&corpus, first.case.seed, true).unwrap();
            assert_eq!(again, first.case);
            assert!(
                planted(&again.materialize(), &first.request)
                    .failures
                    .contains(&first.failure)
            );
        }
        // Refusals of `composite`, grouped by the step that first wrote into it.
        assert!(!refusals.is_empty());
        for (name, members) in refusals {
            assert!(name.ends_with("|composite: not loaded, no note"), "{name}");
            let shrunk = shrunk(&mut ev, &members[0]);
            // Both stage files stay, neither empty, one with the line a step wrote: without
            // them `composite` would be missing, not refused.
            assert_eq!(
                shrunk.files.keys().collect::<Vec<_>>(),
                ["composite.fsh", "composite.vsh"],
                "{name}"
            );
            let texts: Vec<String> = shrunk
                .files
                .values()
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .collect();
            assert!(texts.iter().all(|t| !t.trim().is_empty()), "{name}");
            assert!(texts.iter().any(|t| t.contains("fz_")), "{name}");
            assert!(
                texts.iter().all(|t| t.lines().count() == 1),
                "{name}: {texts:?}"
            );
        }
    }

    #[test]
    fn a_step_on_another_programs_file_expects_nothing() {
        let corpus = cases::corpus(None);
        let base = Outcome {
            loaded: vec!["composite".into(), "final".into()],
            ..Default::default()
        };
        let mut case = smoke(&corpus)
            .into_iter()
            .find(|c| c.programs.len() == 7)
            .unwrap();
        let mut files = case.files.clone();
        files.insert("final.fsh".into(), b"x".to_vec());
        assert_eq!(expect(&case, &base, &files), ["final"]);
        files.insert("lib/common.glsl".into(), b"x".to_vec());
        assert_eq!(expect(&case, &base, &files), Vec::<String>::new());
        case.files.insert(
            "shaders.properties".into(),
            b"program.final.enabled=false".to_vec(),
        );
        let files = case.files.clone();
        assert_eq!(expect(&case, &base, &files), Vec::<String>::new());
    }

    #[test]
    fn worker_lines_round_trip() {
        let outcome = Outcome {
            failures: vec![Failure::new("naga", "composite: x")],
            loaded: vec!["composite".into()],
            shadow_pass: true,
        };
        assert_eq!(parse_outcome(&outcome_json(&outcome)), Some(outcome));
    }
}
