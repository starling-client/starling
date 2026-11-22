use std::{collections::BTreeMap, fs};

const CORPUS: &str = "tests/content-corpus";

pub fn entries() -> BTreeMap<String, String> {
    let mut paths = BTreeMap::new();
    for entry in fs::read_dir(CORPUS).unwrap_or_else(|e| panic!("{CORPUS}: {e}")) {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        let filename = entry.file_name().into_string().expect("UTF-8 fixture name");
        let name = if kind.is_dir() {
            filename.as_str()
        } else if let Some(name) = filename.strip_suffix(".smod").filter(|_| kind.is_file()) {
            name
        } else {
            panic!("{CORPUS}/{filename} is not a folder or a .smod file");
        };
        assert!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "invalid fixture name: {CORPUS}/{filename}"
        );
        // The literal slash keeps Content's source path identical on every platform.
        let previous = paths.insert(name.to_owned(), format!("{CORPUS}/{filename}"));
        assert!(previous.is_none(), "duplicate corpus entry: {name}");
    }
    assert!(!paths.is_empty(), "{CORPUS} is empty");
    paths
}
