//! Starling's tests of startup_menu.rs, out of Sunrise's file (tools/starling/move_out.py).
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

#[test]
fn retry_after_a_delivered_failure_reads_the_new_loader_result() {
    let (sender, receiver) = std::sync::mpsc::channel();
    sender
        .send(LoaderMsg::Done(Err("first failure".into())))
        .unwrap();
    drop(sender);
    let mut loading = loading(receiver);
    loading.poll_menu_resources().unwrap();
    assert!(loading.loaded.take().unwrap().is_err());
    loading.failure = Some("first failure".into());
    let scratch = std::env::temp_dir().join(format!(
        "starling-retry-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    loading.vanilla_root = scratch.clone();
    loading.options =
        Options::parse(["--resources".to_owned(), scratch.display().to_string()].into_iter())
            .unwrap();
    loading.retry().unwrap();
    loading.handle.take().unwrap().join().unwrap();
    loading.poll_menu_resources().unwrap();
    assert!(
        matches!(loading.loaded, Some(Err(_))),
        "the replacement loader's result must be delivered even if the first attempt failed"
    );
}

#[test]
fn a_new_loader_is_read_after_a_failed_generation() {
    // No game files yet: the first generation fails, and the setup shows.
    let (sender, receiver) = std::sync::mpsc::channel();
    sender
        .send(LoaderMsg::Done(Err("no game files".into())))
        .unwrap();
    let mut loading = loading(receiver);
    loading.poll_menu_resources().unwrap();
    assert!(matches!(loading.loaded.take(), Some(Err(e)) if e == "no game files"));
    // Prepared: a loader of its own, whose result has to arrive (Loading::retry).
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        sender
            .send(LoaderMsg::Done(Err("second generation".into())))
            .unwrap();
    });
    worker.join().unwrap();
    loading.follow_loader(receiver, std::thread::spawn(|| {}));
    loading.poll_menu_resources().unwrap();
    assert!(matches!(loading.loaded.take(), Some(Err(e)) if e == "second generation"));
}
