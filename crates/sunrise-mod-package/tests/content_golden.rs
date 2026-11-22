//! Content declarations admitted (or refused) within a stable release line.
//! To add an entry, put its folder or `.smod` in the corpus and run just the `content_golden` test
//! (`-- --exact content_golden`) with `SUNRISE_BLESS_CONTENT_GOLDEN=1`. It only writes missing files.

#[path = "support/content_corpus.rs"]
mod corpus;

use std::{
    env, fs,
    io::{self, Write},
    path::Path,
};
use sunrise_mod_package::{Content, Package};

const GOLDEN: &str = "tests/content-golden";

/// The dump names each entry by its path from the crate root, with literal forward slashes
/// (Content's Source::Dir or Source::Zip), wherever the test executable was started.
fn at_crate_root() {
    env::set_current_dir(env!("CARGO_MANIFEST_DIR")).unwrap();
}

/// The same two checks for the daily run, whose green a stable release needs (`verify` runs
/// the plain ones on every rollup and main push).
#[test]
#[ignore = "daily"]
fn daily_content_golden() {
    content_golden();
    content_golden_is_deterministic();
}

#[test]
fn content_golden() {
    at_crate_root();
    let entries = corpus::entries();
    let bless = env::var("SUNRISE_BLESS_CONTENT_GOLDEN").as_deref() == Ok("1");
    let mut orphans = Vec::new();
    for golden in fs::read_dir(GOLDEN).unwrap_or_else(|e| panic!("{GOLDEN}: {e}")) {
        let golden = golden.unwrap();
        let file = golden.file_name().into_string().expect("UTF-8 golden name");
        assert!(
            golden.file_type().unwrap().is_file(),
            "{GOLDEN}/{file} is not a regular file"
        );
        let name = file.strip_suffix(".txt").expect("golden file ends in .txt");
        if !entries.contains_key(name) {
            orphans.push(file);
        }
    }
    orphans.sort();
    let mut mismatches = Vec::new();
    let mut first_diff = None;

    for (entry, path) in &entries {
        let actual = dump(Path::new(path));
        let golden = format!("{GOLDEN}/{entry}.txt");
        if bless {
            // Creating is the check: an existing golden is never opened for writing.
            match fs::File::create_new(&golden) {
                Ok(mut file) => {
                    file.write_all(actual.as_bytes())
                        .unwrap_or_else(|e| panic!("{golden}: {e}"));
                    continue;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => panic!("{golden}: {e}"),
            }
        }
        let expected = match fs::read(&golden) {
            Ok(bytes) if bytes == actual.as_bytes() => continue,
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => "(missing)\n".into(),
            Err(e) => panic!("{golden}: {e}"),
        };
        mismatches.push(format!("{entry}\n"));
        first_diff.get_or_insert_with(|| {
            format!("{entry} changed\n--- golden ---\n{expected}--- actual ---\n{actual}")
        });
    }

    assert!(
        orphans.is_empty(),
        "goldens without corpus entries: {orphans:?}"
    );
    if let Some(diff) = first_diff {
        panic!(
            "content golden mismatches:\n{}\n{diff}",
            mismatches.concat()
        );
    }
}

#[test]
fn content_golden_is_deterministic() {
    at_crate_root();
    for (entry, path) in corpus::entries() {
        let path = Path::new(&path);
        assert_eq!(dump(path), dump(path), "{entry} changed between reads");
    }
}

fn dump(path: &Path) -> String {
    let Package { manifest, icon } = match Package::read(path) {
        Ok(package) => package,
        Err(error) => return format!("manifest:\nrefused: {error}\n\nicon:\nunavailable\n"),
    };
    let content = Content::read(path, &manifest).and_then(|content| {
        match content.server_only_refusal(&manifest) {
            Some(error) => Err(error),
            None => Ok(content),
        }
    });
    let content = match content {
        Ok(content) => format!("{content:#?}"),
        Err(error) => format!("refused: {error}"),
    };
    format!("manifest:\n{manifest:#?}\n\nicon:\n{icon:?}\n\ncontent:\n{content}\n")
}
