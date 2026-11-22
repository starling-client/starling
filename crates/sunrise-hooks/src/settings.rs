//! The settings store (docs/layers.md, "Platform registries"): a crate above the engine keeps its
//! own settings as `key:value` lines in the game folder, next to `options.txt`, which stays
//! exactly vanilla's. Its lines go in `sunrise-extras.txt` or `sunrise-graphics.txt` (a setting
//! that moved out of the engine's extras or video settings keeps its key there, so players keep
//! their choice) or in a file of its own.
//!
//! The engine routes the lines when it loads and saves the player's settings, never per frame.
//! With no store registered it reads and writes nothing more than before.
use std::fs;
use std::io;
use std::path::Path;

use crate::TooLate;
use crate::registry::Registry;

/// One crate's settings. The values live in the crate (statics); the engine only hands it its
/// lines and asks for them back.
pub trait Store: Sync {
    /// Its file in the game folder: `sunrise-extras.txt` or `sunrise-graphics.txt`, or its own
    /// `sunrise-<name>.txt` that the engine doesn't write.
    fn file(&self) -> &'static str;
    /// Whether a line's key is this store's.
    fn owns(&self, key: &str) -> bool;
    /// One of its lines (key and value trimmed), as the game loads the settings. A value it can't
    /// read keeps the setting as it was.
    fn load(&self, key: &str, value: &str);
    /// Its lines, as the game saves the settings.
    fn save(&self, line: &mut dyn FnMut(&str, &str));
    /// Whether every setting is at its default: then a file that doesn't exist yet isn't made.
    fn is_default(&self) -> bool;
}

static STORES: Registry<dyn Store> = Registry::new();

/// Adds `store` to those the game loads and saves. Before the game starts, as
/// [`crate::register`].
pub fn register(store: &'static dyn Store) -> Result<(), TooLate> {
    STORES.register(store)
}

pub(crate) fn all() -> &'static [&'static dyn Store] {
    STORES.all()
}

/// The engine's side: where it loads and saves the stores. Only the engine calls these.
pub mod engine {
    use super::*;

    /// Loads every store. `read(file)` is the text of a file of the game folder, if it exists;
    /// the engine passes the text it already read for a file it reads itself.
    pub fn load(read: &mut dyn FnMut(&str) -> Option<String>) {
        load_into(all(), read);
    }

    /// Appends the stores' lines of `file`, a file the engine writes itself, to `text`, its own
    /// lines. True when every store of that file is at its defaults (none counts).
    pub fn append(file: &str, text: &mut String) -> bool {
        append_lines(all(), file, text)
    }

    /// Writes the stores' own files in `dir`: every file of a store but `engine_files`, the ones
    /// the engine writes with [`append`]. A line of the file no store wrote now is kept. The files
    /// that could not be read or written come back with why; each is left as it was.
    pub fn save_own(dir: &Path, engine_files: &[&str]) -> Vec<(&'static str, io::Error)> {
        save_files(all(), dir, engine_files)
    }

    /// A settings file's text, `None` when there is none. Bytes that aren't UTF-8 read as
    /// U+FFFD, as vanilla's `Options.load` reads them, so every other line loads; such a file is
    /// `lossy`, and is never written over, so the bytes it couldn't decode stay as they are.
    pub fn read(path: &Path) -> io::Result<Option<FileText>> {
        match fs::read(path) {
            Ok(bytes) => Ok(Some(match String::from_utf8(bytes) {
                Ok(text) => FileText { text, lossy: false },
                Err(e) => FileText {
                    text: String::from_utf8_lossy(e.as_bytes()).into_owned(),
                    lossy: true,
                },
            })),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Replaces `path` with `text` in one step: a new sibling temporary file, synced, then renamed
    /// over it, so a failure or an interruption leaves the old file whole.
    pub fn replace(path: &Path, text: &str) -> io::Result<()> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("settings")
            .to_owned();
        let pid = std::process::id();
        let names = std::iter::repeat_with(|| NEXT.fetch_add(1, Ordering::Relaxed))
            .map(|n| path.with_file_name(format!(".{name}.{pid}.{n}.tmp")));
        replace_through(path, text, names.take(16))
    }

    /// `replace` through the first of `temps` that doesn't exist yet: one that does is left alone.
    pub(super) fn replace_through(
        path: &Path,
        text: &str,
        temps: impl IntoIterator<Item = std::path::PathBuf>,
    ) -> io::Result<()> {
        use std::io::Write;
        let mut taken = io::Error::new(io::ErrorKind::AlreadyExists, "no free temporary name");
        for temp in temps {
            let mut file = match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
            {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    taken = e;
                    continue;
                }
                Err(e) => return Err(e),
            };
            let result = file
                .write_all(text.as_bytes())
                .and_then(|()| file.sync_all())
                .and_then(|()| fs::rename(&temp, path));
            if result.is_err() {
                let _ = fs::remove_file(&temp);
            }
            return result;
        }
        Err(taken)
    }

    /// The keys of `text`'s `key:value` lines: what the stores wrote.
    pub fn keys(text: &str) -> Vec<&str> {
        lines(text).map(|(_, key, _)| key).collect()
    }
}

/// A settings file's text as [`engine::read`] gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileText {
    pub text: String,
    /// Some bytes weren't UTF-8 and read as U+FFFD: the file must not be written over.
    pub lossy: bool,
}

/// Each file the stores use, once, in registration order.
fn files(stores: &[&dyn Store]) -> Vec<&'static str> {
    let mut files: Vec<&'static str> = Vec::new();
    for store in stores {
        if !files.contains(&store.file()) {
            files.push(store.file());
        }
    }
    files
}

/// `key:value` lines, as the game's settings files have them.
fn lines(text: &str) -> impl Iterator<Item = (&str, &str, &str)> {
    text.lines().filter_map(|line| {
        let (key, value) = line.split_once(':')?;
        Some((line, key.trim(), value.trim()))
    })
}

fn load_into(stores: &[&dyn Store], read: &mut dyn FnMut(&str) -> Option<String>) {
    for file in files(stores) {
        let Some(text) = read(file) else {
            continue;
        };
        for (_, key, value) in lines(&text) {
            for store in stores.iter().filter(|s| s.file() == file && s.owns(key)) {
                store.load(key, value);
            }
        }
    }
}

fn append_lines(stores: &[&dyn Store], file: &str, text: &mut String) -> bool {
    let mut default = true;
    for store in stores.iter().filter(|s| s.file() == file) {
        default &= store.is_default();
        store.save(&mut |key, value| {
            text.push_str(key);
            text.push(':');
            text.push_str(value);
            text.push('\n');
        });
    }
    default
}

fn save_files(
    stores: &[&dyn Store],
    dir: &Path,
    engine_files: &[&str],
) -> Vec<(&'static str, io::Error)> {
    let mut failed = Vec::new();
    for file in files(stores) {
        if engine_files.contains(&file) {
            continue;
        }
        let path = dir.join(file);
        let existing = match engine::read(&path) {
            Ok(Some(read)) if read.lossy => {
                let why = "it has bytes that aren't UTF-8, so it is left as it is";
                failed.push((file, io::Error::new(io::ErrorKind::InvalidData, why)));
                continue;
            }
            Ok(existing) => existing.map(|read| read.text),
            Err(e) => {
                failed.push((file, e));
                continue;
            }
        };
        let mut text = String::new();
        let default = append_lines(stores, file, &mut text);
        if existing.is_none() && default {
            continue;
        }
        // Lines no store wrote now, as written: another build's, a setting whose crate isn't in
        // this one, one a store left alone this time, or one that isn't `key:value` at all.
        let written = engine::keys(&text);
        let mut kept = String::new();
        for record in existing
            .as_deref()
            .unwrap_or_default()
            .split_inclusive('\n')
        {
            let key = record.split_once(':').map(|(key, _)| key.trim());
            if !key.is_some_and(|key| written.contains(&key)) {
                kept.push_str(record);
            }
        }
        text.push_str(&kept);
        if let Err(e) = engine::replace(&path, &text) {
            failed.push((file, e));
        }
    }
    failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A store of one switch, `<prefix>.on`, off by default.
    struct Switch {
        file: &'static str,
        key: &'static str,
        on: Mutex<bool>,
    }

    impl Switch {
        const fn new(file: &'static str, key: &'static str) -> Self {
            Self {
                file,
                key,
                on: Mutex::new(false),
            }
        }
        fn on(&self) -> bool {
            *self.on.lock().unwrap()
        }
        fn set(&self, on: bool) {
            *self.on.lock().unwrap() = on;
        }
    }

    impl Store for Switch {
        fn file(&self) -> &'static str {
            self.file
        }
        fn owns(&self, key: &str) -> bool {
            key == self.key
        }
        fn load(&self, _key: &str, value: &str) {
            match value {
                "true" => self.set(true),
                "false" => self.set(false),
                _ => {}
            }
        }
        fn save(&self, line: &mut dyn FnMut(&str, &str)) {
            line(self.key, if self.on() { "true" } else { "false" });
        }
        fn is_default(&self) -> bool {
            !self.on()
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sunrise-hooks-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn each_store_reads_only_its_own_lines() {
        let zoom = Switch::new("sunrise-extras.txt", "zoom");
        let pad = Switch::new("sunrise-pad.txt", "rumble");
        let stores: [&dyn Store; 2] = [&zoom, &pad];
        let mut asked = Vec::new();
        load_into(&stores, &mut |file| {
            asked.push(file.to_owned());
            match file {
                // The engine's own lines, and a key the store's name merely starts.
                "sunrise-extras.txt" => Some("sort:true\nzoom : true\nzoomLevel:4\n".into()),
                _ => Some("zoom:true\nrumble:maybe\n".into()),
            }
        });
        assert!(zoom.on());
        // `rumble:maybe` is no value it reads, and `zoom` in its file isn't its key.
        assert!(!pad.on());
        assert_eq!(
            asked,
            ["sunrise-extras.txt", "sunrise-pad.txt"],
            "each file read once"
        );
    }

    #[test]
    fn a_shared_file_gets_the_stores_lines_after_the_engines() {
        let zoom = Switch::new("sunrise-extras.txt", "zoom");
        let pad = Switch::new("sunrise-pad.txt", "rumble");
        let stores: [&dyn Store; 2] = [&zoom, &pad];
        let mut text = "sort:true\n".to_owned();
        assert!(append_lines(&stores, "sunrise-extras.txt", &mut text));
        assert_eq!(text, "sort:true\nzoom:false\n");
        zoom.set(true);
        let mut text = String::new();
        assert!(!append_lines(&stores, "sunrise-extras.txt", &mut text));
        assert_eq!(text, "zoom:true\n");
    }

    #[test]
    fn own_files_appear_once_changed_and_keep_lines_nobody_owns() {
        let dir = scratch("own-files");
        let pad = Switch::new("sunrise-pad.txt", "rumble");
        let zoom = Switch::new("sunrise-extras.txt", "zoom");
        let stores: [&dyn Store; 2] = [&pad, &zoom];
        save_files(&stores, &dir, &["sunrise-extras.txt"]);
        assert!(
            !dir.join("sunrise-pad.txt").exists(),
            "defaults make no file"
        );
        assert!(
            !dir.join("sunrise-extras.txt").exists(),
            "the engine writes that one"
        );
        pad.set(true);
        save_files(&stores, &dir, &["sunrise-extras.txt"]);
        assert_eq!(
            fs::read_to_string(dir.join("sunrise-pad.txt")).unwrap(),
            "rumble:true\n"
        );
        // Another build's line stays; the store's own line is rewritten, not repeated.
        fs::write(dir.join("sunrise-pad.txt"), "rumble:true\ndeadzone:0.2\n").unwrap();
        pad.set(false);
        save_files(&stores, &dir, &["sunrise-extras.txt"]);
        assert_eq!(
            fs::read_to_string(dir.join("sunrise-pad.txt")).unwrap(),
            "rumble:false\ndeadzone:0.2\n"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// Owns `presence` but writes it only on the first seat, as a per-seat store does.
    struct FirstSeatOnly;
    impl Store for FirstSeatOnly {
        fn file(&self) -> &'static str {
            "sunrise-pad.txt"
        }
        fn owns(&self, key: &str) -> bool {
            key == "presence"
        }
        fn load(&self, _: &str, _: &str) {}
        fn save(&self, _: &mut dyn FnMut(&str, &str)) {}
        fn is_default(&self) -> bool {
            false
        }
    }

    #[test]
    fn a_line_its_store_did_not_write_this_time_is_kept() {
        let dir = scratch("unwritten");
        fs::write(dir.join("sunrise-pad.txt"), "presence:true\n").unwrap();
        let stores: [&dyn Store; 1] = [&FirstSeatOnly];
        assert!(save_files(&stores, &dir, &[]).is_empty());
        assert_eq!(
            fs::read_to_string(dir.join("sunrise-pad.txt")).unwrap(),
            "presence:true\n"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_replace_leaves_the_old_file_and_no_temporary() {
        let dir = scratch("replace");
        let target = dir.join("sunrise-pad.txt");
        // A directory in the file's place: the rename onto it fails.
        fs::create_dir_all(target.join("kept")).unwrap();
        assert!(engine::replace(&target, "rumble:true\n").is_err());
        assert!(target.join("kept").is_dir());
        let left: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, [std::ffi::OsString::from("sunrise-pad.txt")]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_with_a_bad_byte_loads_its_other_lines_and_is_never_written_over() {
        let dir = scratch("bad-byte");
        let path = dir.join("sunrise-pad.txt");
        let bytes = b"rumble:false\nname:\xff\nno colon \xfe\r\n";
        fs::write(&path, bytes).unwrap();
        let read = engine::read(&path).unwrap().unwrap();
        assert!(read.lossy && read.text.starts_with("rumble:false\n"));
        assert_eq!(engine::read(&dir.join("absent.txt")).unwrap(), None);
        let pad = Switch::new("sunrise-pad.txt", "rumble");
        pad.set(true);
        let stores: [&dyn Store; 1] = [&pad];
        assert_eq!(save_files(&stores, &dir, &[]).len(), 1, "the save says why");
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn kept_records_are_saved_as_written_the_last_one_unterminated() {
        let dir = scratch("kept-records");
        let path = dir.join("sunrise-pad.txt");
        fs::write(&path, "rumble:false\r\nno colon here\r\nother : 2 ").unwrap();
        let pad = Switch::new("sunrise-pad.txt", "rumble");
        pad.set(true);
        let stores: [&dyn Store; 1] = [&pad];
        assert!(save_files(&stores, &dir, &[]).is_empty());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "rumble:true\nno colon here\r\nother : 2 "
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_at_the_temporary_name_is_left_alone() {
        let dir = scratch("temp-taken");
        let target = dir.join("sunrise-pad.txt");
        let (taken, free) = (dir.join(".taken.tmp"), dir.join(".free.tmp"));
        fs::write(&taken, "someone else's").unwrap();
        engine::replace_through(&target, "rumble:true\n", [taken.clone(), free.clone()]).unwrap();
        assert_eq!(fs::read_to_string(&taken).unwrap(), "someone else's");
        assert_eq!(fs::read_to_string(&target).unwrap(), "rumble:true\n");
        assert!(!free.exists());
        let _ = fs::remove_dir_all(dir);
    }
}
