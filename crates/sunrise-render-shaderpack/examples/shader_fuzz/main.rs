//! The Rust side of tools/shader_fuzz.py: a seeded robustness fuzzer of the shader-pack
//! translator. Each case is a pack (the committed seed pack, or a pack of `--corpus`), whole, with
//! one or two of its programs mutated by a few named steps (`cases::generate`),
//! and loaded through `load_post` as the game loads a pack, with the game's own inputs
//! (`pack_atlas::game_inputs`).
//!
//!     shader_fuzz [--seed N] [--count N] [--budget SECONDS] [--corpus DIR] [--jobs N]
//!                 [--replay SEED|smoke:N] [--smoke] [--time-limit SECONDS] [--memory-limit MIB]
//!                 [--size-limit MIB] [--shrink-seconds N] [--work-dir DIR] [--seed-pack DIR]
//!
//! Every case keeps these invariants: no panic; done within `--time-limit`; within
//! `--memory-limit` (the worker's allocator refuses past it, which aborts it); every stage file
//! of the dimension's folder within `--size-limit` once preprocessed; every program that loads validates in naga; every
//! note names its subject, and a program the unmutated case loaded is loaded or named; the same
//! case loads the same twice.
//!
//! The run (this process) only lists the corpus: names and paths. Each of `--jobs` case
//! processes (this binary, `case`) opens and reads the packs of its cases, makes them and
//! shrinks them, within `--memory-limit`, so a pack that exhausts it or hangs costs its case,
//! reported as that case's failure. A case process loads each candidate in its own eval
//! worker (this binary, `worker`), so a hang or an abort of the translator costs one load. A
//! failure is grouped by invariant, first failing step and detail; each group's smallest case
//! is shrunk while the same failure stays, written as a pack folder under `<work-dir>/groups/`,
//! and listed in `<work-dir>/report.json` with the arguments that replay its smallest case
//! (`replay_args`), beside the cases run and skipped (each skipped one is printed with why) and
//! how each case process ended. `--budget` alone generates cases until it ends, `--count` alone
//! that many, both until the first is reached, neither 200. Exits 1 when any case fails, or
//! when no case ran. `--seed-pack` replaces the committed seed pack
//! (`examples/shader_fuzz/seeds`).

mod cases;
mod check;
mod process;
mod shrink;

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use cases::{Case, CaseId, Entry};
use check::{Failure, Outcome, Request};
use process::Process;
use shrink::{Budget, Evaluate, Files, Found, Info};

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
    /// Cases to generate: none past this many. Unbounded when only `--budget` is given; 200
    /// when neither is.
    count: Option<u64>,
    /// Seconds to generate cases for: none starts past it.
    budget: Option<Duration>,
    corpus: Option<PathBuf>,
    jobs: usize,
    replay: Option<CaseId>,
    smoke: bool,
    time_limit: Duration,
    memory_mib: usize,
    size_limit: usize,
    shrink: Duration,
    work: PathBuf,
    /// The pack the smoke cases take, and the index's first.
    seeds: PathBuf,
    /// This binary, which runs the case and eval processes.
    exe: PathBuf,
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        seed: 1,
        count: None,
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
        seeds: cases::seed_pack(),
        exe: std::env::current_exe().unwrap_or_default(),
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value")).cloned();
        let number = |v: String| v.parse::<u64>().map_err(|_| format!("{arg}: not a number"));
        match arg.as_str() {
            "--seed" => o.seed = number(value()?)?,
            "--count" => o.count = Some(number(value()?)?),
            "--budget" => o.budget = Some(Duration::from_secs(number(value()?)?)),
            "--corpus" => o.corpus = Some(PathBuf::from(value()?)),
            "--jobs" => o.jobs = number(value()?)?.max(1) as usize,
            "--replay" => {
                let id = value()?;
                let parsed = id
                    .parse()
                    .map_err(|()| format!("{arg}: not a seed or smoke:N"))?;
                o.replay = Some(parsed);
            }
            "--smoke" => o.smoke = true,
            "--time-limit" => o.time_limit = Duration::from_secs(number(value()?)?),
            "--memory-limit" => o.memory_mib = number(value()?)? as usize,
            "--size-limit" => o.size_limit = (number(value()?)? as usize) << 20,
            "--shrink-seconds" => o.shrink = Duration::from_secs(number(value()?)?),
            "--work-dir" => o.work = PathBuf::from(value()?),
            "--seed-pack" => o.seeds = PathBuf::from(value()?),
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    if o.count.is_none() && o.budget.is_none() {
        o.count = Some(200);
    }
    Ok(o)
}

/// The arguments that make case `id` of run `o` again, alone (`--replay`), with what decides
/// its failures: the seed pack when the run had its own (`--seed-pack`: the smoke cases are its
/// programs, and it is the first pack a generated case draws from), the corpus a generated case
/// draws its pack from (the smoke cases take the seed pack only), and the run's time, memory and
/// size limits.
fn replay_args(o: &Options, id: CaseId) -> Vec<String> {
    let mut out = vec!["--replay".to_owned(), id.to_string()];
    if o.seeds != cases::seed_pack() {
        out.push("--seed-pack".to_owned());
        out.push(o.seeds.to_string_lossy().into_owned());
    }
    if let (CaseId::Seed(_), Some(corpus)) = (id, &o.corpus) {
        out.push("--corpus".to_owned());
        out.push(corpus.to_string_lossy().into_owned());
    }
    for (flag, value) in [
        ("--time-limit", o.time_limit.as_secs()),
        ("--memory-limit", o.memory_mib as u64),
        ("--size-limit", (o.size_limit >> 20) as u64),
    ] {
        out.push(flag.to_owned());
        out.push(value.to_string());
    }
    out
}

/// `args` as one POSIX shell line: each argument as it is when it holds only characters the
/// shell takes literally, else in single quotes (a `'` in it written `'"'"'`).
fn shell_line(args: &[String]) -> String {
    let plain = |a: &str| {
        !a.is_empty()
            && a.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"@%+=:,./_-".contains(&b))
    };
    let quoted: Vec<String> = args
        .iter()
        .map(|a| {
            if plain(a) {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', "'\"'\"'"))
            }
        })
        .collect();
    quoted.join(" ")
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

/// A path of `files` that can't be written below the run's shader directory.
fn unstageable(files: &Files) -> Option<&String> {
    files.keys().find(|p| !p.split('/').all(safe_component))
}

/// Validate the whole pack before replacing any previous scratch data.
fn write_pack(dir: &Path, files: &Files) -> std::io::Result<()> {
    if let Some(path) = unstageable(files) {
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

fn request_value(r: &Request) -> serde_json::Value {
    serde_json::json!({"dim": r.dim, "flat": r.flat, "expect": r.expect, "size_limit": r.size_limit})
}

fn strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

fn parse_request(v: &serde_json::Value) -> Request {
    Request {
        dim: v["dim"].as_u64().unwrap_or(0) as usize,
        flat: v["flat"].as_bool().unwrap_or(true),
        expect: strings(&v["expect"]),
        size_limit: v["size_limit"].as_u64().unwrap_or(u64::MAX) as usize,
    }
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
    let failures = v["failures"]
        .as_array()?
        .iter()
        .map(|f| match &strings(f)[..] {
            [invariant, detail] => Some(Failure::new(invariant, detail.clone())),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Outcome {
        failures,
        loaded: strings(&v["loaded"]),
        shadow_pass: v["shadow_pass"].as_bool()?,
    })
}

fn id_value(id: CaseId) -> serde_json::Value {
    match id {
        CaseId::Seed(seed) => serde_json::json!({"seed": seed}),
        CaseId::Smoke(n) => serde_json::json!({"smoke": n}),
    }
}

fn parse_id(v: &serde_json::Value) -> Option<CaseId> {
    match (v["seed"].as_u64(), v["smoke"].as_u64()) {
        (Some(seed), None) => Some(CaseId::Seed(seed)),
        (None, Some(n)) => Some(CaseId::Smoke(n as usize)),
        _ => None,
    }
}

fn info_value(i: &Info) -> serde_json::Value {
    serde_json::json!({
        "seed": i.seed, "pack": i.pack, "dim": i.dim, "flat": i.flat, "programs": i.programs,
        "bytes": i.bytes, "steps": i.steps,
    })
}

fn parse_info(v: &serde_json::Value) -> Option<Info> {
    Some(Info {
        seed: v["seed"].as_u64()?,
        pack: v["pack"].as_str()?.to_owned(),
        dim: v["dim"]
            .as_u64()
            .map(|d| d as usize)
            .filter(|&d| d < cases::DIMENSIONS.len()),
        flat: v["flat"].as_bool(),
        programs: strings(&v["programs"]),
        bytes: v["bytes"].as_u64()? as usize,
        steps: v["steps"].as_u64()? as usize,
    })
}

fn found_value(f: &Found) -> serde_json::Value {
    serde_json::json!({
        "id": id_value(f.id), "case": info_value(&f.case), "request": request_value(&f.request),
        "failure": [f.failure.invariant, f.failure.detail], "step": f.step,
    })
}

fn parse_found(v: &serde_json::Value) -> Option<Found> {
    let failure = match &strings(&v["failure"])[..] {
        [invariant, detail] => Failure::new(invariant, detail.clone()),
        _ => return None,
    };
    Some(Found {
        id: parse_id(&v["id"])?,
        case: parse_info(&v["case"])?,
        request: parse_request(&v["request"]),
        failure,
        step: v["step"].as_str()?.to_owned(),
    })
}

/// The eval worker: a case's pack folder and request in, its outcome out, a line each.
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
        let dir = PathBuf::from(v["dir"].as_str().unwrap_or("."));
        let outcome = check::check(&dir, &parse_request(&v["request"]));
        let _ = writeln!(out, "{}", outcome_json(&outcome));
        let _ = out.flush();
    }
}

/// The eval worker of a case process: each candidate pack written to `dir` and checked there.
struct Worker {
    dir: PathBuf,
    memory_mib: usize,
    time_limit: Duration,
    process: Process,
}

impl Worker {
    fn new(exe: &Path, dir: PathBuf, memory_mib: usize, time_limit: Duration) -> Self {
        let args = ["worker".to_owned(), memory_mib.to_string()];
        Self {
            dir,
            memory_mib,
            time_limit,
            process: Process::new(exe, &args, None),
        }
    }
}

/// A pack a case process could not write to its folder (a file name the run can't write, a
/// file where the pack also has a folder of that name, a write that failed): the case's, not a
/// translator finding. Raised as a panic payload through the case's run, caught by `staged`.
struct Staging(String);

/// `files` written as the pack in `dir`, or `Staging` raised.
fn stage(dir: &Path, files: &Files) {
    if let Err(e) = write_pack(dir, files) {
        std::panic::panic_any(Staging(format!("{}: {e}", dir.display())));
    }
}

/// `run`'s result, or why a pack it wrote could not be staged.
fn staged<T>(run: impl FnOnce() -> T) -> Result<T, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(value) => Ok(value),
        Err(payload) => match payload.downcast::<Staging>() {
            Ok(staging) => Err(staging.0),
            Err(payload) => std::panic::resume_unwind(payload),
        },
    }
}

impl Evaluate for Worker {
    fn eval(&mut self, files: &Files, request: &Request) -> Outcome {
        stage(&self.dir, files);
        let line = serde_json::json!({"dir": self.dir, "request": request_value(request)});
        let reply = self.process.ask(&line.to_string(), self.time_limit);
        if let Ok(line) = &reply
            && let Some(outcome) = parse_outcome(line)
        {
            return outcome;
        }
        let failure = self.process.fail(&reply, self.time_limit, self.memory_mib);
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
    // Of the files the programs read, and the root `.properties` files: a step that renames a
    // name runs through every source file of the pack, which holds the other programs' too.
    let touched: Vec<&String> = case
        .files
        .keys()
        .filter(|p| case.reach.contains(*p) || (!p.contains('/') && p.ends_with(".properties")))
        .filter(|p| files.get(*p) != case.files.get(*p))
        .collect();
    base.loaded
        .iter()
        .filter(|name| touched.iter().all(|t| case.owns(name, t)))
        .cloned()
        .collect()
}

/// A case's pack, dimension and terrain: its unmutated load (the pack is whole whatever programs
/// the steps edit).
type BaseKey = (String, usize, bool);

/// Runs made cases through `ev`, keeping each unmutated case's outcome.
struct Runner<E> {
    ev: E,
    bases: HashMap<BaseKey, Outcome>,
    size_limit: usize,
}

impl<E: Evaluate> Runner<E> {
    fn new(ev: E, size_limit: usize) -> Self {
        Self {
            ev,
            bases: HashMap::new(),
            size_limit,
        }
    }

    fn request(&self, case: &Case, expect: Vec<String>) -> Request {
        Request {
            dim: case.dim,
            flat: case.flat,
            expect,
            size_limit: self.size_limit,
        }
    }

    /// Case `id` (`case`)'s failures, each with its first failing step.
    fn run(&mut self, id: CaseId, case: &Case) -> Vec<Found> {
        let key = (case.pack.clone(), case.dim, case.flat);
        let base = match self.bases.get(&key) {
            Some(base) => base.clone(),
            None => {
                let base = self.ev.eval(&case.files, &self.request(case, Vec::new()));
                self.bases.insert(key, base.clone());
                base
            }
        };
        let files = case.materialize();
        let request = self.request(case, expect(case, &base, &files));
        let outcome = self.ev.eval(&files, &request);
        let info = Info::of(case, &files);
        outcome
            .failures
            .into_iter()
            .map(|failure| Found {
                id,
                case: info.clone(),
                step: shrink::first_step(&mut self.ev, case, &request, &failure.signature()),
                request: request.clone(),
                failure,
            })
            .collect()
    }
}

/// What a group's shrunk case holds: its steps, its files and their lines.
#[derive(Debug, PartialEq)]
struct Summary {
    steps: Vec<String>,
    files: Vec<String>,
    lines: usize,
}

impl Summary {
    fn of(shrunk: &shrink::Shrunk) -> Self {
        Self {
            steps: shrunk
                .steps
                .iter()
                .map(|s| format!("{} {}", s.kind.name(), s.file))
                .collect(),
            files: shrunk.files.keys().cloned().collect(),
            lines: shrunk
                .files
                .values()
                .map(|b| b.split(|&c| c == b'\n').filter(|l| !l.is_empty()).count())
                .sum(),
        }
    }
}

/// What running a case gave.
#[derive(Debug, PartialEq)]
enum Ran {
    /// Its failures.
    Found(Vec<Found>),
    /// It makes no case (`cases::NoCase`), or its pack could not be staged: why.
    Skipped(String),
    /// A smoke case past the last (or past a seed pack that doesn't open), why: the smoke cases
    /// end there; a replay of it is skipped, with why.
    Past(String),
}

impl From<cases::NoCase> for Ran {
    fn from(no: cases::NoCase) -> Self {
        match no {
            cases::NoCase::Past(why) => Ran::Past(why),
            no => Ran::Skipped(no.to_string()),
        }
    }
}

/// Where a run's cases are made and run: case processes in a run, this process in tests.
trait Cases {
    fn run(&mut self, id: CaseId) -> Ran;

    /// `found`'s case shrunk within `seconds` while it fails the same way, written as a pack in
    /// `dir`; `None` when it can't be made again.
    fn shrink(&mut self, found: &Found, seconds: Duration, dir: &Path) -> Option<Summary>;
}

/// Cases made, read and run in this process: what a case process does, and the tests.
struct Local<'a, E> {
    index: &'a [Entry],
    runner: Runner<E>,
}

impl<E: Evaluate> Cases for Local<'_, E> {
    fn run(&mut self, id: CaseId) -> Ran {
        match cases::sample(self.index, id) {
            Ok(case) => Ran::Found(self.runner.run(id, &case)),
            Err(no) => no.into(),
        }
    }

    fn shrink(&mut self, found: &Found, seconds: Duration, dir: &Path) -> Option<Summary> {
        let case = cases::sample(self.index, found.id).ok()?;
        let budget = Budget {
            evals: 4000,
            until: Instant::now() + seconds,
        };
        let signature = found.failure.signature();
        let shrunk = shrink::shrink(
            &mut self.runner.ev,
            &case,
            &found.request,
            &signature,
            budget,
        );
        stage(dir, &shrunk.files);
        Some(Summary::of(&shrunk))
    }
}

/// A case process: the run's index and limits first, then cases to make and run, or a group's
/// case to shrink. Each case's pack is opened and read here, within `--memory-limit`, so a pack
/// that exhausts it ends this process and is that case's failure, not the run's.
fn case_process() {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let Some(Ok(config)) = lines.next() else {
        return;
    };
    let config: serde_json::Value = serde_json::from_str(&config).expect("a configuration");
    let index: Vec<Entry> = config["index"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            Some(Entry {
                name: e[0].as_str()?.to_owned(),
                path: PathBuf::from(e[1].as_str()?),
            })
        })
        .collect();
    let memory_mib = config["memory_mib"].as_u64().unwrap_or(2048) as usize;
    let time_limit = Duration::from_secs(config["time_limit"].as_u64().unwrap_or(20));
    let size_limit = config["size_limit"].as_u64().unwrap_or(u64::MAX) as usize;
    let dir = PathBuf::from(config["dir"].as_str().expect("a worker directory"));
    LIMIT.store(memory_mib << 20, Ordering::Relaxed);
    // A pack that can't be staged is answered (`staged`), not a crash to report.
    let report = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if info.payload().downcast_ref::<Staging>().is_none() {
            report(info);
        }
    }));
    let exe = std::env::current_exe().expect("this binary");
    let ev = Worker::new(&exe, dir, memory_mib, time_limit);
    let mut local = Local {
        index: &index,
        runner: Runner::new(ev, size_limit),
    };
    let mut out = std::io::stdout();
    let mut reply = |v: serde_json::Value| {
        let _ = writeln!(out, "{v}");
        let _ = out.flush();
    };
    for line in lines {
        let Ok(line) = line else { break };
        let v: serde_json::Value = serde_json::from_str(&line).expect("a request");
        if let Some(found) = parse_found(&v["shrink"]) {
            let seconds = Duration::from_secs(v["seconds"].as_u64().unwrap_or(0));
            let dir = PathBuf::from(v["dir"].as_str().expect("a group directory"));
            let summary = match staged(|| local.shrink(&found, seconds, &dir)) {
                Ok(summary) => summary.map(
                    |s| serde_json::json!({"steps": s.steps, "files": s.files, "lines": s.lines}),
                ),
                Err(why) => {
                    reply(serde_json::json!({"shrunk": null, "staging": why}));
                    continue;
                }
            };
            reply(serde_json::json!({ "shrunk": summary }));
            continue;
        }
        let id = parse_id(&v["run"]).expect("a case");
        let case = match cases::sample(&index, id) {
            Ok(case) => case,
            Err(cases::NoCase::Past(why)) => {
                reply(serde_json::json!({ "past": why }));
                continue;
            }
            Err(no) => {
                reply(serde_json::json!({ "skipped": no.to_string() }));
                continue;
            }
        };
        reply(serde_json::json!({"sampled": info_value(&Info::sampled(&case))}));
        // A pack the run can't write makes no case: the driver's limit, not the translator's.
        match staged(|| local.runner.run(id, &case)) {
            Ok(found) => reply(
                serde_json::json!({"found": found.iter().map(found_value).collect::<Vec<_>>()}),
            ),
            Err(why) => reply(
                serde_json::json!({ "skipped": format!("{}: can't stage the pack: {why}", case.pack) }),
            ),
        }
    }
    // The run's requests ended: the eval worker's end too, waited for, and how it ended.
    let worker = local.runner.ev.process.close().map(|c| c.status);
    reply(serde_json::json!({ "closed": worker }));
}

/// A case process of the run, from the coordinator's side.
struct Remote<'a> {
    index: &'a [Entry],
    o: &'a Options,
    process: Process,
}

impl<'a> Remote<'a> {
    fn new(index: &'a [Entry], o: &'a Options, n: usize) -> Self {
        let config = serde_json::json!({
            "index": index.iter().map(|e| (&e.name, &e.path)).collect::<Vec<_>>(),
            "memory_mib": o.memory_mib,
            "time_limit": o.time_limit.as_secs(),
            "size_limit": o.size_limit,
            "dir": o.work.join(format!("worker{n}")),
        });
        Self {
            index,
            o,
            process: Process::new(&o.exe, &["case".to_owned()], Some(config.to_string())),
        }
    }

    /// Case `id`, whose process ended (`reply`) at `step`: the case's failure.
    fn died(
        &mut self,
        id: CaseId,
        case: Option<Info>,
        reply: &Result<String, RecvTimeoutError>,
        limit: Duration,
        step: &str,
    ) -> Found {
        let failure = self.process.fail(reply, limit, self.o.memory_mib);
        let case = case.unwrap_or_else(|| Info {
            seed: id.seed(),
            pack: self.index[id.pack(self.index.len())].name.clone(),
            dim: None,
            flat: None,
            programs: Vec::new(),
            bytes: 0,
            steps: 0,
        });
        Found {
            id,
            request: Request {
                dim: case.dim.unwrap_or(0),
                flat: case.flat.unwrap_or(true),
                expect: Vec::new(),
                size_limit: self.o.size_limit,
            },
            case,
            failure,
            step: step.to_owned(),
        }
    }
}

impl Remote<'_> {
    /// The case process closed at the end of its cases: how it and its eval worker ended, as
    /// the report lists them (`None` when it never started).
    fn finish(mut self) -> Option<serde_json::Value> {
        let closed = self.process.close()?;
        let worker = closed
            .lines
            .iter()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .find_map(|v| v.get("closed").cloned());
        Some(serde_json::json!({"exit": closed.status, "worker": worker}))
    }
}

impl Cases for Remote<'_> {
    fn run(&mut self, id: CaseId) -> Ran {
        let limit = self.o.time_limit;
        let request = serde_json::json!({"run": id_value(id)}).to_string();
        let reply = self.process.ask(&request, limit);
        let sampled = reply
            .as_ref()
            .ok()
            .and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok());
        let info = match &sampled {
            Some(v) if v["past"].is_string() => {
                return Ran::Past(v["past"].as_str().unwrap_or("").to_owned());
            }
            Some(v) if v["skipped"].is_string() => {
                return Ran::Skipped(v["skipped"].as_str().unwrap_or("").to_owned());
            }
            Some(v) => parse_info(&v["sampled"]),
            None => None,
        };
        let Some(info) = info else {
            return Ran::Found(vec![self.died(id, None, &reply, limit, "sample")]);
        };
        // The base and the case, then the steps one by one: each eval is bounded by the
        // worker's limit, so this bounds only a case process that stopped answering.
        let limit = limit * (info.steps as u32 + 3) + Duration::from_secs(30);
        let reply = self.process.next(limit);
        let answer = reply
            .as_ref()
            .ok()
            .and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok());
        if let Some(why) = answer.as_ref().and_then(|v| v["skipped"].as_str()) {
            return Ran::Skipped(why.to_owned());
        }
        let found = answer.and_then(|v| {
            v["found"]
                .as_array()
                .map(|f| f.iter().map(parse_found).collect::<Option<Vec<Found>>>())
        });
        match found {
            Some(Some(found)) => Ran::Found(found),
            _ => Ran::Found(vec![self.died(id, Some(info), &reply, limit, "run")]),
        }
    }

    fn shrink(&mut self, found: &Found, seconds: Duration, dir: &Path) -> Option<Summary> {
        let request = serde_json::json!({
            "shrink": found_value(found), "seconds": seconds.as_secs(), "dir": dir,
        });
        let limit = seconds + self.o.time_limit * 3 + Duration::from_secs(30);
        let reply = self.process.ask(&request.to_string(), limit);
        let shrunk = reply
            .as_ref()
            .ok()
            .and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok());
        let Some(v) = shrunk else {
            let _ = self.process.fail(&reply, limit, self.o.memory_mib);
            return None;
        };
        if let Some(why) = v["staging"].as_str() {
            eprintln!(
                "shader_fuzz: {}: can't stage the shrunk case: {why}",
                found.case.pack
            );
        }
        let s = &v["shrunk"];
        Some(Summary {
            steps: strings(&s["steps"]),
            files: strings(&s["files"]),
            lines: s["lines"].as_u64()? as usize,
        })
    }
}

/// What every thread of a run shares.
struct Run<'a> {
    o: &'a Options,
    next: AtomicU64,
    started: Instant,
    found: Mutex<Vec<Found>>,
    done: AtomicU64,
    skipped: AtomicU64,
}

impl<'a> Run<'a> {
    fn new(o: &'a Options) -> Self {
        Self {
            o,
            next: AtomicU64::new(0),
            started: Instant::now(),
            found: Mutex::new(Vec::new()),
            done: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
        }
    }

    /// The run's cases through `cases`, until none are left: the smoke cases, the case
    /// `--replay` names, or generated ones until `--count` of them or `--budget` ends,
    /// whichever comes first.
    fn cases(&self, cases: &mut dyn Cases) {
        loop {
            let i = self.next.fetch_add(1, Ordering::Relaxed);
            let id = if self.o.smoke {
                CaseId::Smoke(i as usize)
            } else if let Some(id) = self.o.replay {
                if i > 0 {
                    break;
                }
                id
            } else {
                let over = self.o.budget.is_some_and(|b| self.started.elapsed() >= b);
                if self.o.count.is_some_and(|count| i >= count) || over {
                    break;
                }
                CaseId::Seed(cases::case_seed(self.o.seed, i))
            };
            let found = match cases.run(id) {
                Ran::Found(found) => found,
                // Past the last of `--smoke`'s cases, they end; the one case a replay names is
                // skipped, with why, as any other.
                Ran::Past(_) if self.o.replay.is_none() => break,
                Ran::Skipped(why) | Ran::Past(why) => {
                    eprintln!("shader_fuzz: case {id}: {why}; skipped");
                    self.skipped.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            };
            self.found.lock().unwrap().extend(found);
            let done = self.done.fetch_add(1, Ordering::Relaxed) + 1;
            if done.is_multiple_of(100) {
                println!(
                    "{done} cases, {} failures, {:.0} s",
                    self.found.lock().unwrap().len(),
                    self.started.elapsed().as_secs_f64()
                );
            }
        }
    }
}

/// A group's report entry; its shrunk case (`shrunk`, written in `dir` by the shrinker) and
/// `case.txt` in `dir`.
fn write_group(
    o: &Options,
    dir: &Path,
    name: &str,
    members: &[Found],
    shrunk: Option<&Summary>,
) -> serde_json::Value {
    let first = &members[0];
    let replay = replay_args(o, first.id);
    output(dir, std::fs::create_dir_all(dir));
    let (steps, files, lines) = match shrunk {
        Some(s) => (s.steps.clone(), s.files.clone(), s.lines),
        None => (Vec::new(), Vec::new(), 0),
    };
    let dimension = first.case.dim.map(|d| cases::DIMENSIONS[d].0);
    let terrain = first.case.flat.map(|f| if f { "flat" } else { "grid" });
    let summary = format!(
        "group: {name}\nreplay: {}\npack: {}\nprograms: {}\ndimension: {}\nterrain: {}\nsteps: {}\n",
        shell_line(&replay),
        first.case.pack,
        first.case.programs.join(" "),
        dimension.unwrap_or("?"),
        terrain.unwrap_or("?"),
        if shrunk.is_some() {
            steps.join(", ")
        } else {
            "? (the case could not be made again to shrink)".to_owned()
        },
    );
    let summary_path = dir.join("case.txt");
    output(&summary_path, std::fs::write(&summary_path, summary));
    serde_json::json!({
        "group": name,
        "invariant": first.failure.invariant,
        "step": first.step,
        "detail": first.failure.detail,
        "count": members.len(),
        "cases": members.iter().take(20).map(|m| m.id.to_string()).collect::<Vec<_>>(),
        "replay": first.id.to_string(),
        "replay_args": replay,
        "pack": first.case.pack,
        "programs": first.case.programs,
        "dimension": dimension,
        "flat": first.case.flat,
        "steps": steps,
        "files": files,
        "lines": lines,
        "dir": dir,
    })
}

/// The run `o` asks for, its report written: the exit code, 1 when any case fails.
fn fuzz(o: &Options) -> i32 {
    let mut index = cases::index(&o.seeds, o.corpus.as_deref());
    // A case process is told its packs' paths as text.
    index.retain(|e| {
        let text = e.path.to_str().is_some();
        if !text {
            eprintln!(
                "shader_fuzz: corpus: {}: not a UTF-8 path, left out",
                e.path.display()
            );
        }
        text
    });
    let names: Vec<&str> = index.iter().map(|s| s.name.as_str()).collect();
    println!("corpus: {}", names.join(", "));
    let run = Run::new(o);
    // How each case process ended once its cases did: closed, and waited for.
    let processes = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for n in 0..o.jobs {
            let (run, processes, mut cases) = (&run, &processes, Remote::new(&index, o, n));
            scope.spawn(move || {
                run.cases(&mut cases);
                processes.lock().unwrap().extend(cases.finish());
            });
        }
    });
    let elapsed = run.started.elapsed();
    let found = run.found.into_inner().unwrap();
    let failing = found.len();
    let groups: Vec<(String, Vec<Found>)> = shrink::group(found).into_iter().collect();
    // Each group's smallest case, shrunk; the groups share the case processes.
    let next = AtomicUsize::new(0);
    let entries: Mutex<BTreeMap<usize, serde_json::Value>> = Mutex::new(BTreeMap::new());
    std::thread::scope(|scope| {
        for n in 0..o.jobs.min(groups.len()) {
            let (groups, next, entries, processes) = (&groups, &next, &entries, &processes);
            let mut cases = Remote::new(&index, o, n);
            scope.spawn(move || {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((name, members)) = groups.get(i) else {
                        break;
                    };
                    let group = format!("{:02}-{}", i + 1, members[0].failure.invariant);
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
                    let shrunk = cases.shrink(&members[0], o.shrink, &dir);
                    let entry = write_group(o, &dir, name, members, shrunk.as_ref());
                    entries.lock().unwrap().insert(i, entry);
                }
                processes.lock().unwrap().extend(cases.finish());
            });
        }
    });
    let entries: Vec<serde_json::Value> = entries.into_inner().unwrap().into_values().collect();
    let (done, skipped) = (
        run.done.load(Ordering::Relaxed),
        run.skipped.load(Ordering::Relaxed),
    );
    let report = serde_json::json!({
        "seed": o.seed,
        "cases": done,
        "skipped": skipped,
        "processes": processes.into_inner().unwrap(),
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
        "{done} cases ({skipped} skipped) in {:.0} s: {failing} failures in {} groups",
        elapsed.as_secs_f64(),
        entries.len()
    );
    for g in &entries {
        println!(
            "  {:4}  {}  (replay: {}, {} lines in {})",
            g["count"],
            g["group"].as_str().unwrap_or(""),
            shell_line(&strings(&g["replay_args"])),
            g["lines"],
            g["dir"].as_str().unwrap_or("")
        );
    }
    println!("report: {}", path.display());
    // A run that ran no case checked nothing: not a pass.
    if done == 0 {
        println!("no case ran");
    }
    i32::from(!entries.is_empty() || done == 0)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("worker") => {
            let mib = args.get(1).and_then(|m| m.parse().ok()).unwrap_or(2048);
            return worker(mib);
        }
        Some("case") => return case_process(),
        _ => {}
    }
    let mut o = match options(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("shader_fuzz: {e}");
            std::process::exit(2);
        }
    };
    // The replay commands name the corpus and a given seed pack as paths from anywhere.
    let full = |path: PathBuf| match std::fs::canonicalize(&path) {
        Ok(full) if full.to_str().is_some() => full,
        _ => path,
    };
    o.corpus = o.corpus.map(full);
    if args.iter().any(|arg| arg == "--seed-pack") {
        o.seeds = full(o.seeds);
    }
    if !args.iter().any(|arg| arg == "--work-dir") {
        o.work = output(&o.work, default_folder(&o.work));
    }
    output(&o.work, claim_folder(&o.work));
    std::process::exit(fuzz(&o));
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

    fn local<'a, E: Evaluate>(index: &'a [Entry], ev: E) -> Local<'a, E> {
        Local {
            index,
            runner: Runner::new(ev, 16 << 20),
        }
    }

    /// A folder pack at `dir` with `files` in its `shaders` folder.
    fn write_folder_pack(dir: &Path, files: &[(&str, &[u8])]) {
        for (path, bytes) in files {
            let file = dir.join("shaders").join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
    }

    #[test]
    fn the_seed_pack_holds_every_invariant() {
        let index = cases::index(&cases::seed_pack(), None);
        let dir = temp();
        let mut ev = InProcess { dir: dir.clone() };
        let all = cases::smoke(&index).unwrap();
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

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| (*a).to_owned()).collect()
    }

    /// The cases a run of `list` generates through cases that pass at once, until the run
    /// stops or `fake_end` were made (then the fake ends it, as past the last smoke case does).
    fn generated(list: &[&str], fake_end: usize) -> Vec<CaseId> {
        struct Fake(Vec<CaseId>, usize);
        impl Cases for Fake {
            fn run(&mut self, id: CaseId) -> Ran {
                if self.0.len() == self.1 {
                    return Ran::Past(String::new());
                }
                self.0.push(id);
                Ran::Found(Vec::new())
            }

            fn shrink(&mut self, _: &Found, _: Duration, _: &Path) -> Option<Summary> {
                None
            }
        }
        let o = options(&args(list)).unwrap();
        let run = Run::new(&o);
        let mut fake = Fake(Vec::new(), fake_end);
        run.cases(&mut fake);
        assert_eq!(run.done.load(Ordering::Relaxed), fake.0.len() as u64);
        fake.0
    }

    /// `--budget` alone is not capped by a count: the run goes on until the budget ends (here
    /// an hour, so the fake ends it at 500). 200 cases only when neither control is given.
    #[test]
    fn budget_alone_runs_until_the_budget_ends() {
        let seeds = |n: u64| {
            (0..n)
                .map(|i| CaseId::Seed(cases::case_seed(1, i)))
                .collect()
        };
        let ids: Vec<CaseId> = seeds(500);
        assert_eq!(generated(&["--budget", "3600"], 500), ids);
        assert_eq!(generated(&[], 500), seeds(200));
        assert_eq!(generated(&["--count", "7"], 500), seeds(7));
        assert_eq!(
            generated(&["--count", "7", "--budget", "3600"], 500),
            seeds(7)
        );
        assert_eq!(generated(&["--budget", "0"], 500), []);
        // A budget already spent stops a run whatever its count.
        let o = options(&args(&["--count", "9", "--budget", "5"])).unwrap();
        let mut run = Run::new(&o);
        run.started = Instant::now().checked_sub(Duration::from_secs(6)).unwrap();
        struct Never;
        impl Cases for Never {
            fn run(&mut self, id: CaseId) -> Ran {
                panic!("{id} made past the budget")
            }

            fn shrink(&mut self, _: &Found, _: Duration, _: &Path) -> Option<Summary> {
                None
            }
        }
        run.cases(&mut Never);
        assert_eq!(run.done.load(Ordering::Relaxed), 0);
    }

    /// `--replay` takes what a report gives: a generated case's seed, or a smoke case by its
    /// place; anything else is refused.
    #[test]
    fn replay_names_a_generated_or_a_smoke_case() {
        let replay = |id: &str| options(&args(&["--replay", id])).map(|o| o.replay);
        assert_eq!(replay("0"), Ok(Some(CaseId::Seed(0))));
        assert_eq!(
            replay("18446744073709551615"),
            Ok(Some(CaseId::Seed(u64::MAX)))
        );
        assert_eq!(replay("smoke:3"), Ok(Some(CaseId::Smoke(3))));
        for bad in [
            "",
            "smoke:",
            "smoke:x",
            "smoke:3:4",
            "smoke:-1",
            "-1",
            "+1",
            "0x1",
            " 1",
            "1 ",
            "18446744073709551616",
            "Smoke:1",
            "seed:1",
        ] {
            assert!(replay(bad).is_err(), "{bad:?}");
        }
        assert_eq!(CaseId::Seed(1234).to_string(), "1234");
        assert_eq!(CaseId::Smoke(5).to_string(), "smoke:5");
        // The smoke case replayed is the smoke run's own, not the case seed 0 generates.
        let index = cases::index(&cases::seed_pack(), None);
        let smoke = cases::smoke(&index).unwrap();
        assert!(smoke.len() > 3);
        for (n, case) in smoke.iter().enumerate() {
            assert_eq!(
                cases::sample(&index, CaseId::Smoke(n)).as_ref().ok(),
                Some(case)
            );
        }
        let seed0 = cases::sample(&index, CaseId::Seed(0)).unwrap();
        assert!(!seed0.steps.is_empty());
        assert!(smoke.iter().all(|c| c.steps.is_empty()));
    }

    /// A group's replay arguments keep what decides its case and its failures: the corpus a
    /// generated case is drawn from (not a smoke case's: the seed pack only), the run's own seed
    /// pack (both kinds: the smoke cases' programs, a generated case's first pack), and the run's
    /// time, memory and size limits, defaults or not. The line quotes what the shell would split.
    #[test]
    fn replay_arguments_keep_the_corpus_and_the_limits() {
        let o = options(&args(&[
            "--corpus",
            "/packs/my packs",
            "--time-limit",
            "31",
            "--memory-limit",
            "777",
            "--size-limit",
            "5",
            "--seed",
            "9",
            "--jobs",
            "3",
        ]))
        .unwrap();
        let seed = replay_args(&o, CaseId::Seed(42));
        assert_eq!(
            seed,
            args(&[
                "--replay",
                "42",
                "--corpus",
                "/packs/my packs",
                "--time-limit",
                "31",
                "--memory-limit",
                "777",
                "--size-limit",
                "5"
            ])
        );
        assert_eq!(
            shell_line(&seed),
            "--replay 42 --corpus '/packs/my packs' --time-limit 31 --memory-limit 777 \
             --size-limit 5"
        );
        assert_eq!(
            replay_args(&o, CaseId::Smoke(2)),
            args(&[
                "--replay",
                "smoke:2",
                "--time-limit",
                "31",
                "--memory-limit",
                "777",
                "--size-limit",
                "5"
            ])
        );
        // Replayed, the arguments are the run's case and limits again.
        let again = options(&seed).unwrap();
        assert_eq!(again.replay, Some(CaseId::Seed(42)));
        assert_eq!(again.corpus, o.corpus);
        assert_eq!(
            (again.time_limit, again.memory_mib, again.size_limit),
            (o.time_limit, o.memory_mib, o.size_limit)
        );
        assert_eq!(
            shell_line(&args(&[
                "it's",
                "$HOME",
                "a\"b",
                "",
                "plain-1.0/x=y,z:@%+_"
            ])),
            r#"'it'"'"'s' '$HOME' 'a"b' '' plain-1.0/x=y,z:@%+_"#
        );
        // A run's own seed pack is in every case's arguments, and replays as that seed pack.
        let own = options(&args(&[
            "--seed-pack",
            "/packs/my seeds",
            "--corpus",
            "/packs/c",
        ]))
        .unwrap();
        let seed = replay_args(&own, CaseId::Seed(7));
        assert_eq!(
            seed,
            args(&[
                "--replay",
                "7",
                "--seed-pack",
                "/packs/my seeds",
                "--corpus",
                "/packs/c",
                "--time-limit",
                "20",
                "--memory-limit",
                "2048",
                "--size-limit",
                "16"
            ])
        );
        assert_eq!(
            replay_args(&own, CaseId::Smoke(0)),
            args(&[
                "--replay",
                "smoke:0",
                "--seed-pack",
                "/packs/my seeds",
                "--time-limit",
                "20",
                "--memory-limit",
                "2048",
                "--size-limit",
                "16"
            ])
        );
        assert_eq!(
            options(&seed).unwrap().seeds,
            PathBuf::from("/packs/my seeds")
        );
    }

    /// A smoke case that is skipped (its pack could not be staged) doesn't end the smoke cases:
    /// only the first past the last does.
    #[test]
    fn a_skipped_smoke_case_does_not_end_the_run() {
        struct Fake(Vec<CaseId>);
        impl Cases for Fake {
            fn run(&mut self, id: CaseId) -> Ran {
                self.0.push(id);
                match id {
                    CaseId::Smoke(0) => Ran::Skipped("seeds: can't stage the pack".into()),
                    CaseId::Smoke(n) if n < 3 => Ran::Found(Vec::new()),
                    _ => Ran::Past(String::new()),
                }
            }

            fn shrink(&mut self, _: &Found, _: Duration, _: &Path) -> Option<Summary> {
                None
            }
        }
        let mut o = opts(temp());
        o.smoke = true;
        let run = Run::new(&o);
        let mut fake = Fake(Vec::new());
        run.cases(&mut fake);
        assert_eq!(fake.0, (0..4).map(CaseId::Smoke).collect::<Vec<_>>());
        assert_eq!(run.done.load(Ordering::Relaxed), 2);
        assert_eq!(run.skipped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn fixed_seeds_hold_every_invariant() {
        let index = cases::index(&cases::seed_pack(), None);
        let dir = temp();
        let mut o = opts(dir.clone());
        // Run seed 2: cases of every kind of step that hold, quick enough to run in-process
        // (without the worker's time and memory bounds).
        o.seed = 2;
        o.count = Some(4);
        let run = Run::new(&o);
        run.cases(&mut local(&index, InProcess { dir: dir.join("w") }));
        assert_eq!(run.done.load(Ordering::Relaxed), 4);
        let found = run.found.into_inner().unwrap();
        assert!(
            found.is_empty(),
            "{:?}",
            found.iter().map(Found::group).collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The render-driving outputs of a load: every program's modules, interfaces and settings,
    /// the notes, and the pack's textures, buffer sizes, buffer objects, images, custom
    /// uniforms, buffer formats and shadow pass. (`Custom` itself holds hash maps, whose `Debug`
    /// order is not the same twice.)
    fn outputs(post: &sunrise_shaders::Post) -> Vec<String> {
        let seen = check::observe(post);
        let mut out: Vec<String> = seen
            .pieces
            .iter()
            .map(|(label, hash)| format!("{label} {hash}"))
            .collect();
        out.extend([
            format!("{:?}", seen.loaded),
            format!("{:?}", seen.notes),
            format!("{:?}", post.textures),
            format!("{:?}", post.sizes),
            format!("{:?}", post.buffer_objects),
            format!("{:?}", post.images),
            format!("{:?}", post.custom.uniforms().collect::<Vec<_>>()),
            format!("{:?}", post.custom.notes),
            format!("{:?}", post.config),
            format!("{:?}", post.shadow_pass),
        ]);
        out
    }

    /// The pack at `full` loaded as a case on `programs` (dimension 0, flat terrain, no step)
    /// loads it, and as the pack itself loads: both, with their render-driving outputs equal.
    fn unmutated(full: &Path, programs: &[&str]) -> sunrise_shaders::Post {
        let source = cases::Source::open(full).unwrap();
        let programs = programs.iter().map(|p| (*p).to_owned()).collect();
        let case = Case::unmutated(&source, 0, 0, true, programs);
        let sample = full.with_extension("case");
        write_pack(&sample, &case.materialize()).unwrap();
        let request = Request {
            dim: 0,
            flat: true,
            expect: Vec::new(),
            size_limit: 16 << 20,
        };
        let (whole, part) = (check::load(full, &request), check::load(&sample, &request));
        assert_eq!(outputs(&part), outputs(&whole));
        part
    }

    /// The loader reads, for `composite`: an include with a comment after it, a raw custom
    /// texture and its `.mcmeta`, a buffer object's file, and an option that a stage file of
    /// another (incomplete) program declares, which `shaders.properties` tests. A case on
    /// `composite` keeps them all, and loads exactly as the whole pack does.
    #[test]
    fn an_unmutated_sample_loads_as_the_full_pack() {
        let dir = temp();
        let full = dir.join("full");
        let properties: &[u8] = b"\
customTexture.fzNoise = tex/fz.dat TEXTURE_2D RGBA8 2 2 RGBA UNSIGNED_BYTE
bufferObject.1 = 16 data/fz.bin
#ifdef FZ_HALF
size.buffer.colortex2 = 0.5 0.5
#endif
";
        let composite = b"#version 120

#include \"/lib/a.glsl\" // the shared part

uniform sampler2D colortex0;
varying vec2 texcoord;

void main() {
#ifdef FZ_HALF
    gl_FragData[0] = texture2D(colortex0, texcoord) * fz_a();
#endif
}
";
        let vertex: &[u8] = b"#version 120

varying vec2 texcoord;

void main() {
    gl_Position = ftransform();
    texcoord = gl_MultiTexCoord0.xy;
}
";
        let water_fragment = b"#version 120

#define FZ_HALF

uniform sampler2D colortex0;
varying vec2 texcoord;

void main() {
    gl_FragData[0] = texture2D(colortex0, texcoord);
}
";
        write_folder_pack(
            &full,
            &[
                ("shaders.properties", properties),
                ("composite.vsh", vertex),
                ("composite.fsh", composite),
                ("lib/a.glsl", b"float fz_a() { return 0.5; }\n"),
                ("gbuffers_water.fsh", water_fragment),
                ("tex/fz.dat", &[7u8; 16]),
                ("tex/fz.dat.mcmeta", br#"{"texture": {"blur": true}}"#),
                ("data/fz.bin", &[1u8; 16]),
            ],
        );
        let part = unmutated(&full, &["composite"]);
        // What each input gave: `composite` loaded with its include, the texture blurred, the
        // buffer filled, colortex2 at half size.
        assert_eq!(check::observe(&part).loaded, ["composite"]);
        assert_eq!(part.textures.len(), 1);
        assert_eq!(part.textures[0].name, "fzNoise");
        let sunrise_shaders::TextureSource::Image(texture) = &part.textures[0].source else {
            panic!("a texture from the pack's file");
        };
        assert!(texture.blur);
        assert_eq!(
            part.buffer_objects.get(&1),
            Some(&sunrise_shaders::BufferObject::File {
                bytes: 16,
                path: "data/fz.bin".into(),
                data: vec![1; 16],
            })
        );
        assert_eq!(
            part.sizes[2],
            Some(sunrise_shaders::BufferSize::Relative([0.5, 0.5]))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// What other programs decide for a case's program stays: `composite` sampling
    /// `shadowtex0` turns the shadow pass on (no `shadow.enabled`), so a case on `shadow` loads
    /// it; `gbuffers_terrain_solid` splits terrain, so a case on `gbuffers_terrain` draws the
    /// cutout part with it.
    #[test]
    fn a_sample_keeps_what_the_other_programs_decide() {
        let dir = temp();
        let vertex: &[u8] =
            b"#version 120\nvarying vec2 texcoord;\nvoid main() {\n    gl_Position = ftransform();\n    texcoord = gl_MultiTexCoord0.xy;\n}\n";
        let reads = |sampler: &str| {
            format!(
                "#version 120\nuniform sampler2D {sampler};\nvarying vec2 texcoord;\nvoid main() {{\n    gl_FragData[0] = texture2D({sampler}, texcoord);\n}}\n"
            )
            .into_bytes()
        };
        let shadows = dir.join("shadows");
        let (shadowtex, colortex) = (reads("shadowtex0"), reads("colortex0"));
        write_folder_pack(
            &shadows,
            &[
                ("composite.vsh", vertex),
                ("composite.fsh", &shadowtex),
                ("shadow.vsh", vertex),
                ("shadow.fsh", &colortex),
            ],
        );
        let part = unmutated(&shadows, &["shadow"]);
        assert!(part.shadow_pass);
        assert_eq!(
            part.shadow.as_ref().map(|p| p.name.as_str()),
            Some("shadow")
        );
        let terrain = dir.join("terrain");
        let texture = reads("gtexture");
        write_folder_pack(
            &terrain,
            &[
                ("gbuffers_terrain.vsh", vertex),
                ("gbuffers_terrain.fsh", &texture),
                ("gbuffers_terrain_solid.vsh", vertex),
                ("gbuffers_terrain_solid.fsh", &texture),
            ],
        );
        let part = unmutated(&terrain, &["gbuffers_terrain"]);
        let name = |p: &Option<sunrise_shaders::PostProgram>| p.as_ref().map(|p| p.name.clone());
        assert_eq!(
            name(&part.terrain).as_deref(),
            Some("gbuffers_terrain_solid")
        );
        assert_eq!(
            name(&part.terrain_cutout).as_deref(),
            Some("gbuffers_terrain")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `separateAo = true` gives terrain the game's inputs with each vertex's own ambient
    /// occlusion in its colour's alpha (`PackAtlas::terrain_inputs_sized`'s `separate_ao`).
    #[test]
    fn separate_ao_comes_from_the_pack() {
        let dir = temp();
        let seeds = cases::index(&cases::seed_pack(), None)
            .remove(0)
            .path
            .join("shaders");
        let read = |name: &str| std::fs::read(seeds.join(name)).unwrap();
        let (vertex, fragment) = (read("gbuffers_terrain.vsh"), read("gbuffers_terrain.fsh"));
        write_folder_pack(
            &dir,
            &[
                ("shaders.properties", b"separateAo = true\n"),
                ("gbuffers_terrain.vsh", &vertex),
                ("gbuffers_terrain.fsh", &fragment),
            ],
        );
        let request = Request {
            dim: 0,
            flat: false,
            expect: Vec::new(),
            size_limit: 16 << 20,
        };
        let terrain = |post: &sunrise_shaders::Post| {
            let program = post.terrain.as_ref().expect("terrain loads");
            format!("{:?}", program.program.as_ref().unwrap().vertex)
        };
        let pack = sunrise_shaders::Pack::open(&dir).unwrap();
        let with = |separate_ao: bool| {
            let mut macros = sunrise_shaders::Macros::standard();
            macros.define_renderer(4);
            let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
            let atlas = sunrise_render_shaderpack::pack_atlas::PackAtlas::terrain_inputs_sized(
                [1024, 1024],
                64,
                16,
                separate_ao,
                sunrise_render_shaderpack::pack_atlas::TerrainDraws::Attributes,
            );
            let inputs =
                sunrise_render_shaderpack::pack_atlas::game_inputs(Some(atlas), false, &[3, 4]);
            terrain(&sunrise_shaders::load_post(
                &pack,
                "",
                &properties,
                &macros,
                &inputs,
            ))
        };
        let loaded = terrain(&check::load(&dir, &request));
        assert_eq!(loaded, with(true));
        assert_ne!(loaded, with(false));
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

    fn shrunk(ev: &mut dyn Evaluate, index: &[Entry], first: &Found) -> shrink::Shrunk {
        let budget = Budget {
            evals: 10_000,
            until: Instant::now() + Duration::from_secs(60),
        };
        let case = cases::sample(index, first.id).unwrap();
        let signature = first.failure.signature();
        shrink::shrink(ev, &case, &first.request, &signature, budget)
    }

    #[test]
    fn a_planted_failure_is_shrunk_and_grouped() {
        let index = cases::index(&cases::seed_pack(), None);
        let mut o = opts(temp());
        o.count = Some(1000);
        let run = Run::new(&o);
        let mut ev = planted;
        run.cases(&mut local(&index, ev));
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
            let shrunk = shrunk(&mut ev, &index, first);
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
            let again = cases::generate(&index, first.case.seed, true).unwrap();
            assert_eq!(Ok(&again), cases::sample(&index, first.id).as_ref());
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
            let shrunk = shrunk(&mut ev, &index, &members[0]);
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
        let index = cases::index(&cases::seed_pack(), None);
        let base = Outcome {
            loaded: vec!["composite".into(), "final".into()],
            ..Default::default()
        };
        let mut case = cases::smoke(&index)
            .unwrap()
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
    fn process_lines_round_trip() {
        let outcome = Outcome {
            failures: vec![Failure::new("naga", "composite: x")],
            loaded: vec!["composite".into()],
            shadow_pass: true,
        };
        assert_eq!(parse_outcome(&outcome_json(&outcome)), Some(outcome));
        for (id, case) in [
            (
                CaseId::Seed(u64::MAX),
                Info {
                    seed: u64::MAX,
                    pack: "P \"1\".zip".into(),
                    dim: Some(2),
                    flat: Some(false),
                    programs: vec!["composite1".into(), "final".into()],
                    bytes: 1234,
                    steps: 6,
                },
            ),
            (
                CaseId::Smoke(3),
                Info {
                    seed: 0,
                    pack: "seeds".into(),
                    dim: None,
                    flat: None,
                    programs: Vec::new(),
                    bytes: 0,
                    steps: 0,
                },
            ),
        ] {
            let found = Found {
                id,
                case,
                request: Request {
                    dim: 2,
                    flat: false,
                    expect: vec!["final".into()],
                    size_limit: 16 << 20,
                },
                failure: Failure::new("memory", "over 64 MiB"),
                step: "sample".into(),
            };
            let line = found_value(&found).to_string();
            assert_eq!(
                parse_found(&serde_json::from_str(&line).unwrap()),
                Some(found)
            );
        }
    }
}
