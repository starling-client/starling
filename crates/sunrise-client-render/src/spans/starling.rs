//! Starling's part of spans.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

pub(super) static RECORDING: AtomicBool = AtomicBool::new(false);

pub(super) type Totals = (u64, f64, f64, f32);

pub(super) type Trace = std::collections::BTreeMap<(&'static str, &'static str), Totals>;

pub(super) static TRACE: std::sync::Mutex<Trace> = std::sync::Mutex::new(Trace::new());

/// Opt-in bounded, aggregate timings. Names are engine constants, never world data.
pub fn record_timings(on: bool) {
    DIAGNOSTIC_HERE.with(|h| h.set(on));
    RECORDING.store(on, Ordering::Relaxed);
    TRACE.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

pub(super) fn collect(role: &'static str, span: Span) {
    if !RECORDING.load(Ordering::Relaxed) {
        return;
    }
    let mut trace = TRACE.lock().unwrap_or_else(|e| e.into_inner());
    // Bounds the registry even if future instrumentation adds dynamic stages.
    if trace.len() >= 256 && !trace.contains_key(&(role, span.name)) {
        return;
    }
    let total = trace.entry((role, span.name)).or_default();
    total.0 += 1;
    total.1 += f64::from(span.ms);
    total.2 += f64::from(span.cpu_ms);
    total.3 = total.3.max(span.ms);
}

pub fn drain_timings() -> serde_json::Value {
    let trace = std::mem::take(&mut *TRACE.lock().unwrap_or_else(|e| e.into_inner()));
    serde_json::Value::Array(
        trace
            .into_iter()
            .map(|((role, stage), (calls, wall, cpu, max))| {
                serde_json::json!({"role": role, "stage": stage, "calls": calls,
            "wall_ms": wall, "thread_cpu_ms": cpu, "max_ms": max})
            })
            .collect(),
    )
}

/// A worker task measured independently of the render/server thread's spans.
pub fn job<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    if !RECORDING.load(Ordering::Relaxed) {
        return f();
    }
    let (started, cpu) = (Instant::now(), thread_cpu());
    let value = f();
    let ms = started.elapsed().as_secs_f32() * 1000.0;
    let cpu_ms = thread_cpu()
        .zip(cpu)
        .map_or(ms, |(end, start)| (end - start).as_secs_f32() * 1000.0);
    let thread = std::thread::current();
    let role = if thread.name() == Some("Server thread") {
        "server"
    } else {
        "worker"
    };
    collect(
        role,
        Span {
            name,
            ms,
            cpu_ms,
            depth: 0,
        },
    );
    value
}

/// Records `ms` under `name`, measured by the caller (the sum of a loop's parts); its wall time
/// stands for its CPU time.
pub fn add(name: &'static str, ms: f32) {
    if recording_here() {
        emit(Span {
            name,
            ms,
            cpu_ms: ms,
            depth: DEPTH.with(Cell::get),
        });
    }
}

/// Raw spans belong only to threads whose existing consumer drains them. Recording
/// aggregate diagnostics must not grow a worker's thread-local buffer forever.
pub(super) fn emit(span: Span) {
    if RECORDING.load(Ordering::Relaxed) {
        let role = if std::thread::current().name() == Some("Server thread") {
            "server"
        } else if HERE.with(Cell::get) != 0 || DIAGNOSTIC_HERE.with(Cell::get) {
            "render"
        } else {
            "worker"
        };
        collect(role, span);
    }
    if enabled()
        || HERE.with(Cell::get) != 0
        || timeline::on()
        || (RECORDING.load(Ordering::Relaxed) && DIAGNOSTIC_HERE.with(Cell::get))
    {
        push(span);
    }
}
