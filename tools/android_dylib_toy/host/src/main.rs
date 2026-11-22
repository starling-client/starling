//! The toy program: bumps the counter, opens the mod library named on its command line with `dlopen`, calls its
//! entry and its hook, and checks they counted on the same counter (one std, one game) and that a box made in the
//! mod drops here. Prints `toy: ok` and exits 0 when they did.
use std::ffi::{CString, c_char, c_int, c_void};

unsafe extern "C" {
    fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

const RTLD_NOW: c_int = 2;

fn main() {
    let path = std::env::args().nth(1).expect("usage: toyhost LIBRARY");
    let first = toygame::bump();
    let file = CString::new(path.clone()).unwrap();
    // SAFETY: a library this test built, opened once and kept for the process.
    let handle = unsafe { dlopen(file.as_ptr(), RTLD_NOW) };
    if handle.is_null() {
        // SAFETY: dlerror's message for the failed dlopen above.
        let why = unsafe { std::ffi::CStr::from_ptr(dlerror()) };
        eprintln!("toy: dlopen {path}: {}", why.to_string_lossy());
        std::process::exit(1);
    }
    let name = CString::new("toy_mod_v1").unwrap();
    // SAFETY: the symbol the mod exports with this signature.
    let entry: fn() -> toygame::Entry = unsafe {
        let symbol = dlsym(handle, name.as_ptr());
        assert!(!symbol.is_null(), "toy: no toy_mod_v1");
        std::mem::transmute::<*mut c_void, fn() -> toygame::Entry>(symbol)
    };
    let loaded = entry();
    let said = loaded.hooks.hello();
    let last = toygame::bump();
    println!("toy: host first {first}, mod {}, said {said:?}, host last {last}", loaded.id);
    drop(loaded);
    // The host bumped twice and the mod twice: 4 only with one shared counter.
    if last == 4 {
        println!("toy: ok");
    } else {
        println!("toy: the mod has its own counter (two std or two games)");
        std::process::exit(2);
    }
}
