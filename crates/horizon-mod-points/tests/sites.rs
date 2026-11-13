//! Real sealed-registry dispatch, with the same typed slot the staging macro emits.
// Generated fixtures exercise every shape; TODO(#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
use mod_points::register;
use mod_points::{
    __private::{SITES, SlotListener, seal},
    SiteId,
};
use std::sync::{Mutex, atomic::Ordering};
const SITE: SiteId = SiteId::S7173cf1fd760a6e4;
static TRACE: Mutex<Vec<u8>> = Mutex::new(Vec::new());
struct Args {
    rate: f32,
}
type Receiver<'g, 'r> = ();
type Output = i64;
type Listener = fn((), &mut Args, &mut dyn FnMut((), &mut Args) -> i64) -> i64;
fn outer(r: (), a: &mut Args, next: &mut dyn FnMut((), &mut Args) -> i64) -> i64 {
    TRACE.lock().unwrap().push(1);
    let result = next(r, a);
    TRACE.lock().unwrap().push(6);
    result + 100
}
fn inner(r: (), a: &mut Args, next: &mut dyn FnMut((), &mut Args) -> i64) -> i64 {
    TRACE.lock().unwrap().push(2);
    a.rate += 1.0;
    let result = next(r, a) + next(r, a);
    TRACE.lock().unwrap().push(5);
    result
}
fn listener(id: &'static str, priority: i32, injection: &'static str, f: Listener) -> SlotListener {
    // SAFETY: this test executable supplies the site's exact typed slot below; no game fires it.
    unsafe { SlotListener::site(id, SITE.place(), priority, injection, f as *const ()) }
}

#[test]
fn site_listeners_nest_repeat_original_and_skip_stopped_mods() {
    assert!(!SITES[SITE as usize].load(Ordering::Relaxed));
    let mut args = Args { rate: 2.0 };
    let mut call = |_: (), a: &mut Args| {
        TRACE.lock().unwrap().push(3);
        (a.rate * 100.0) as i64
    };
    assert_eq!(
        horizon_mod_api_macros::site!(
            __test_site_wrap,
            S7173cf1fd760a6e4,
            Listener,
            Receiver,
            Args,
            Output,
            (),
            args,
            call
        ),
        200
    );
    TRACE.lock().unwrap().clear();
    // Register in reverse order: priority wins, then mod id, then injection id.
    register(&listener("z_low", 0, "inner", inner)).unwrap();
    register(&listener("a_high", 10, "outer", outer)).unwrap();
    register(&listener("a_high", 0, "stopped", outer)).unwrap();
    mod_points::stop("a_high");
    register(&listener("b_high", 10, "outer", outer)).unwrap();
    seal();
    assert!(SITES[SITE as usize].load(Ordering::Relaxed));
    assert_eq!(
        horizon_mod_api_macros::site!(
            __test_site_wrap,
            S7173cf1fd760a6e4,
            Listener,
            Receiver,
            Args,
            Output,
            (),
            args,
            call
        ),
        700
    );
    assert_eq!(*TRACE.lock().unwrap(), [1, 2, 3, 3, 5, 6]);
    assert_eq!(
        register(&listener("late", 0, "late", outer)),
        Err(mod_points::RegisterError::Sealed)
    );
    mod_points::stop("b_high");
    mod_points::stop("z_low");
    assert_eq!(
        horizon_mod_api_macros::site!(
            __test_site_wrap,
            S7173cf1fd760a6e4,
            Listener,
            Receiver,
            Args,
            Output,
            (),
            args,
            call
        ),
        300
    );
}
