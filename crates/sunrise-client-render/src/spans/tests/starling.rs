//! Starling's tests of spans.rs, out of Sunrise's file (tools/starling/move_out.py).
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

#[test]
fn diagnostic_aggregates_do_not_retain_worker_raw_spans() {
    let _switch = spans_switch();
    super::super::record_timings(true);
    super::super::time("render_step", || ());
    assert_eq!(super::super::take_raw()[0].name, "render_step");
    std::thread::spawn(|| {
        for _ in 0..10000 {
            super::super::time("worker_step", || ());
        }
        assert!(super::super::take_raw().is_empty());
    })
    .join()
    .unwrap();
    let rows = super::super::drain_timings();
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .any(|row| row["stage"] == "render_step" && row["role"] == "render")
    );
    let step = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["stage"] == "worker_step")
        .unwrap();
    assert_eq!(step["calls"], 10000);
    assert_eq!(step["role"], "worker");
    super::super::record_timings(false);
}
