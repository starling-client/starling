//! The multiplayer server list, stored like vanilla's ServerList in an
//! uncompressed `servers.dat`: `{servers: [{name, ip, icon?, acceptTextures?,
//! acceptedCodeOfConduct?, hidden}]}`. Hidden entries (servers joined by
//! Direct Connection or quick play) are not shown but keep their settings.
use std::path::Path;
use sunrise_core::{
    nbt,
    nbt::{JavaString, NamedTag, Tag},
};

#[derive(Clone, Debug, PartialEq)]
pub struct ServerEntry {
    pub name: String,
    pub ip: String,
    /// Base64 PNG favicon from the last status ping.
    pub icon: Option<String>,
    /// ServerPackStatus: None = prompt, Some(true) enabled, Some(false) disabled.
    pub accept_textures: Option<bool>,
    /// `String.hashCode` of the Code of Conduct accepted for good (ServerData's
    /// `acceptedCodeOfConduct`); 0 for none.
    pub accepted_code_of_conduct: i32,
}
impl ServerEntry {
    pub fn new(name: &str, ip: &str) -> Self {
        Self {
            name: name.to_owned(),
            ip: ip.to_owned(),
            icon: None,
            accept_textures: None,
            accepted_code_of_conduct: 0,
        }
    }
    fn to_tag(&self) -> Tag {
        let mut entries = vec![
            ("name", Tag::String(JavaString::from(self.name.as_str()))),
            ("ip", Tag::String(JavaString::from(self.ip.as_str()))),
        ];
        if let Some(icon) = &self.icon {
            entries.push(("icon", Tag::String(JavaString::from(icon.as_str()))));
        }
        if let Some(accept) = self.accept_textures {
            entries.push(("acceptTextures", Tag::Byte(i8::from(accept))));
        }
        if self.accepted_code_of_conduct != 0 {
            entries.push((
                "acceptedCodeOfConduct",
                Tag::Int(self.accepted_code_of_conduct),
            ));
        }
        Tag::compound(entries)
    }
    fn from_tag(tag: &Tag) -> Option<Self> {
        if !matches!(tag, Tag::Compound(_)) {
            return None;
        }
        let string = |k: &str| match tag.get(k) {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        };
        Some(Self {
            name: string("name").unwrap_or_default(),
            ip: string("ip").unwrap_or_default(),
            icon: string("icon"),
            // ServerData.ServerPackStatus.FIELD_CODEC -> NbtOps.getBooleanValue:
            // any numeric tag whose doubleValue() != 0.0.
            accept_textures: match tag.get("acceptTextures") {
                Some(Tag::Byte(n)) => Some(f64::from(*n) != 0.0),
                Some(Tag::Short(n)) => Some(f64::from(*n) != 0.0),
                Some(Tag::Int(n)) => Some(f64::from(*n) != 0.0),
                Some(Tag::Long(n)) => Some((*n as f64) != 0.0),
                Some(Tag::Float(bits)) => Some(f64::from(f32::from_bits(*bits)) != 0.0),
                Some(Tag::Double(bits)) => Some(f64::from_bits(*bits) != 0.0),
                _ => None,
            },
            // CompoundTag.getIntOr("acceptedCodeOfConduct", 0) -> NumericTag.intValue().
            accepted_code_of_conduct: match tag.get("acceptedCodeOfConduct") {
                Some(Tag::Byte(n)) => i32::from(*n),
                Some(Tag::Short(n)) => i32::from(*n),
                Some(Tag::Int(n)) => *n,
                Some(Tag::Long(n)) => *n as i32,
                Some(Tag::Float(bits)) => sunrise_core::mth::floor_f32(f32::from_bits(*bits)),
                Some(Tag::Double(bits)) => sunrise_core::mth::floor(f64::from_bits(*bits)),
                _ => 0,
            },
        })
    }
}

/// Held while servers.dat is read and written back, so a ping storing an icon and a list
/// saved by the screen or at startup don't undo each other's change.
pub fn lock() -> std::sync::MutexGuard<'static, ()> {
    static SAVING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SAVING.lock().unwrap_or_else(|e| e.into_inner())
}

/// ServerList.load: the shown entries; missing or unreadable files give an empty list.
pub fn load(path: &Path) -> Vec<ServerEntry> {
    load_all(path).0
}
/// ServerList.load: the shown entries (`serverList`) and the hidden ones
/// (`hiddenServerList`, `hidden` set), each in the file's order.
pub fn load_all(path: &Path) -> (Vec<ServerEntry>, Vec<ServerEntry>) {
    let (mut shown, mut hidden) = (Vec::new(), Vec::new());
    let Ok(bytes) = std::fs::read(path) else {
        return (shown, hidden);
    };
    let Ok((root, _)) = nbt::read_named(&bytes, nbt::Limits::unlimited_heap()) else {
        return (shown, hidden);
    };
    if let Some(Tag::List(list)) = root.tag.get("servers") {
        for t in list {
            let Some(entry) = ServerEntry::from_tag(t) else {
                continue;
            };
            // CompoundTag.getBooleanOr("hidden", false) -> NumericTag.byteValue() != 0.
            let is_hidden = match t.get("hidden") {
                Some(Tag::Byte(b)) => *b != 0,
                Some(Tag::Short(n)) => *n as i8 != 0,
                Some(Tag::Int(n)) => *n as i8 != 0,
                Some(Tag::Long(n)) => *n as i8 != 0,
                Some(Tag::Float(bits)) => {
                    (sunrise_core::mth::floor_f32(f32::from_bits(*bits)) as i8) != 0
                }
                Some(Tag::Double(bits)) => {
                    (sunrise_core::mth::floor(f64::from_bits(*bits)) as i8) != 0
                }
                _ => false,
            };
            if is_hidden {
                hidden.push(entry);
            } else {
                shown.push(entry);
            }
        }
    }
    (shown, hidden)
}
/// ServerList.save for the shown entries, keeping the hidden ones the file has.
pub fn save(path: &Path, servers: &[ServerEntry]) -> std::io::Result<()> {
    let (_, hidden) = load_all(path);
    save_all(path, servers, &hidden)
}
/// ServerList.save: the shown entries with `hidden: false`, then the hidden
/// ones with `hidden: true`, written to a temporary file, then replaced.
pub fn save_all(
    path: &Path,
    servers: &[ServerEntry],
    hidden: &[ServerEntry],
) -> std::io::Result<()> {
    let flagged = |entry: &ServerEntry, flag: bool| {
        let mut tag = entry.to_tag();
        if let Tag::Compound(c) = &mut tag {
            c.insert(JavaString::from("hidden"), Tag::Byte(i8::from(flag)));
        }
        tag
    };
    let list = servers
        .iter()
        .map(|e| flagged(e, false))
        .chain(hidden.iter().map(|e| flagged(e, true)))
        .collect();
    let root = NamedTag {
        name: JavaString::from(""),
        tag: Tag::compound([("servers", Tag::List(list))]),
    };
    write(path, &root)
}
/// ServerList.add(server, true): a hidden entry goes first, and only the 16
/// newest hidden entries are kept (`MAX_HIDDEN_SERVERS`); then saved.
pub fn add_hidden(path: &Path, entry: ServerEntry) -> std::io::Result<()> {
    let (shown, mut hidden) = load_all(path);
    hidden.insert(0, entry);
    hidden.truncate(16);
    save_all(path, &shown, &hidden)
}
/// Puts `entry` first, keeping every other entry and field as the file has them (the hidden
/// ones too). A file that is there but can't be read is left as it is.
pub fn prepend(path: &Path, entry: &ServerEntry) -> std::io::Result<()> {
    insert(path, entry, |_| None)
}
/// Removes every entry `matches` accepts (given its name and address), keeping the other
/// entries and every field as the file has them (the hidden ones too). Whether any went; a
/// missing file has none, and the file is only written when something went.
pub fn remove_where(path: &Path, matches: impl Fn(&str, &str) -> bool) -> std::io::Result<bool> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let mut root = nbt::read_named(&bytes, nbt::Limits::unlimited_heap())
        .map_err(std::io::Error::other)?
        .0;
    let Some(Tag::List(list)) = root.tag.get_mut("servers") else {
        return Ok(false);
    };
    let text = |t: &Tag, key: &str| match t.get(key) {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => String::new(),
    };
    let before = list.len();
    list.retain(|t| !matches(&text(t, "name"), &text(t, "ip")));
    if list.len() == before {
        return Ok(false);
    }
    write(path, &root)?;
    Ok(true)
}
/// Inserts `entry` at the index `at` picks in the file's list (`None`: first).
fn insert(
    path: &Path,
    entry: &ServerEntry,
    at: impl Fn(&[Tag]) -> Option<usize>,
) -> std::io::Result<()> {
    let mut root = match std::fs::read(path) {
        Ok(bytes) => {
            nbt::read_named(&bytes, nbt::Limits::unlimited_heap())
                .map_err(std::io::Error::other)?
                .0
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => NamedTag {
            name: JavaString::from(""),
            tag: Tag::Compound(Default::default()),
        },
        Err(e) => return Err(e),
    };
    let Tag::Compound(top) = &mut root.tag else {
        return Err(std::io::Error::other("servers.dat is not a compound"));
    };
    match top
        .entry(JavaString::from("servers"))
        .or_insert_with(|| Tag::List(Vec::new()))
    {
        Tag::List(list) => {
            let index = at(list).unwrap_or(0);
            list.insert(index, entry.to_tag());
        }
        _ => return Err(std::io::Error::other("servers.dat has no server list")),
    }
    write(path, &root)
}
fn write(path: &Path, root: &NamedTag) -> std::io::Result<()> {
    let bytes =
        nbt::write_named(root, nbt::Limits::unlimited_heap()).map_err(std::io::Error::other)?;
    // ServerList.save + Util.safeReplaceFile: write a temp file, back up the
    // preceding servers.dat to servers.dat_old, then move the new file into place.
    let tmp = path.with_extension(format!("dat_tmp.{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    let mut backup_os = path.as_os_str().to_os_string();
    backup_os.push("_old");
    let backup = std::path::PathBuf::from(backup_os);
    let backed_up = if path.exists() {
        match std::fs::remove_file(&backup) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        }
        if let Err(e) = std::fs::rename(path, &backup) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        true
    } else {
        false
    };
    if let Err(e) = std::fs::rename(&tmp, path) {
        if backed_up {
            let _ = std::fs::rename(&backup, path);
        }
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// ServerAddress.parseString: `host`, `host:port` or `[v6]:port`; default 25565.
pub fn parse_address(text: &str) -> Option<(String, u16)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(rest) = text.strip_prefix('[') {
        let (host, tail) = rest.rsplit_once(']')?;
        // HostAndPort requires a colon inside a bracketed host; it does not
        // accept a bracketed DNS name as an IPv6-style address.
        if !host.contains(':') {
            return None;
        }
        let port = match tail.strip_prefix(':') {
            Some(p) if p.bytes().all(|byte| byte.is_ascii_digit()) => parse_port(p)?,
            None if tail.is_empty() => 25565,
            _ => return None,
        };
        return Some((host.to_owned(), port));
    }
    match text.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => {
            if host.is_empty() {
                return None;
            }
            Some((host.to_owned(), parse_port(port)?))
        }
        Some(_) => Some((text.to_owned(), 25565)),
        None => Some((text.to_owned(), 25565)),
    }
}

/// `ServerList.get`: shown entries, then hidden. A name plus address picks that
/// row; an address alone is the first row with that ip.
pub fn find<'a>(
    shown: &'a [ServerEntry],
    hidden: &'a [ServerEntry],
    address: &str,
    name: Option<&str>,
) -> Option<&'a ServerEntry> {
    let mut rows = shown.iter().chain(hidden.iter());
    match name {
        Some(name) => rows.find(|s| s.ip == address && s.name == name),
        None => rows.find(|s| s.ip == address),
    }
}

/// `ServerList.saveSingleServer` for the resource-pack setting: only the shown
/// or hidden row with this name and address is written.
pub fn save_single_accept_textures(
    shown: &mut [ServerEntry],
    hidden: &mut [ServerEntry],
    name: &str,
    address: &str,
    setting: Option<bool>,
) -> bool {
    if let Some(entry) = shown.iter_mut().find(|s| s.name == name && s.ip == address) {
        entry.accept_textures = setting;
        true
    } else if let Some(entry) = hidden
        .iter_mut()
        .find(|s| s.name == name && s.ip == address)
    {
        entry.accept_textures = setting;
        true
    } else {
        false
    }
}

/// HostAndPort uses the default for an empty port, refuses a leading plus,
/// and accepts signed zero on its non-bracketed path before checking range.
fn parse_port(text: &str) -> Option<u16> {
    if text.is_empty() {
        Some(25565)
    } else if text.starts_with('+') {
        None
    } else {
        u16::try_from(text.parse::<i32>().ok()?).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn servers_dat_round_trips() {
        let dir = std::env::temp_dir().join(format!("sunrise-servers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("servers.dat");
        let list = vec![
            ServerEntry::new("Minecraft Server", "localhost:25565"),
            ServerEntry {
                accept_textures: Some(false),
                ..ServerEntry::new("Other", "example.org")
            },
        ];
        save(&path, &list).unwrap();
        assert_eq!(load(&path), list);
        // Vanilla layout: an unnamed root compound with a "servers" list.
        let (root, _) =
            nbt::read_named(&std::fs::read(&path).unwrap(), nbt::Limits::default()).unwrap();
        assert_eq!(root.name, JavaString::from(""));
        assert!(matches!(root.tag.get("servers"), Some(Tag::List(l)) if l.len() == 2));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prepend_keeps_hidden_entries_and_unknown_fields() {
        let dir = std::env::temp_dir().join(format!("sunrise-prepend-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("servers.dat");
        prepend(&path, &ServerEntry::new("First", "a.example")).unwrap();
        assert_eq!(load(&path), [ServerEntry::new("First", "a.example")]);
        let hidden = Tag::compound([
            ("name", Tag::String(JavaString::from("Direct"))),
            ("ip", Tag::String(JavaString::from("b.example"))),
            ("hidden", Tag::Byte(1)),
            ("acceptedCodeOfConduct", Tag::String(JavaString::from("x"))),
        ]);
        let root = NamedTag {
            name: JavaString::from(""),
            tag: Tag::compound([("servers", Tag::List(vec![hidden.clone()]))]),
        };
        write(&path, &root).unwrap();
        prepend(&path, &ServerEntry::new("Top", "c.example")).unwrap();
        let (root, _) =
            nbt::read_named(&std::fs::read(&path).unwrap(), nbt::Limits::default()).unwrap();
        let Some(Tag::List(list)) = root.tag.get("servers") else {
            panic!("a server list");
        };
        assert_eq!(
            list,
            &[ServerEntry::new("Top", "c.example").to_tag(), hidden]
        );
        // A file that can't be read stays as it is.
        std::fs::write(&path, b"not nbt").unwrap();
        assert!(prepend(&path, &ServerEntry::new("Top", "c.example")).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"not nbt");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parses_addresses_like_server_address() {
        assert_eq!(
            parse_address("localhost"),
            Some(("localhost".into(), 25565))
        );
        assert_eq!(
            parse_address("127.0.0.1:25570"),
            Some(("127.0.0.1".into(), 25570))
        );
        assert_eq!(parse_address("[::1]:1234"), Some(("::1".into(), 1234)));
        assert_eq!(parse_address("host:notaport"), None);
        assert_eq!(parse_address(""), None);
    }

    #[test]
    fn empty_explicit_ports_use_the_vanilla_default() {
        for (input, host) in [
            ("example.com:", "example.com"),
            ("127.0.0.1:", "127.0.0.1"),
            ("[::1]:", "::1"),
            ("[2001:db8::1]:", "2001:db8::1"),
            (" \t[::1]:\r\n", "::1"),
        ] {
            assert_eq!(parse_address(input), Some((host.into(), 25565)), "{input}");
        }
    }

    #[test]
    fn ports_and_brackets_keep_vanilla_validation() {
        for input in [
            ":",
            ":25565",
            "[]",
            "[]:",
            "[example.com]",
            "[example.com]:",
            "[::1]junk",
            "example.com:+25565",
            "[::1]:+25565",
            "[::1]:-0",
            "example.com:-1",
            "example.com:65536",
            "[::1]:65536",
            "host:2147483648",
            "host:1_000",
            "host:\u{0661}\u{0662}",
        ] {
            assert_eq!(parse_address(input), None, "{input}");
        }
        assert_eq!(parse_address("host:-0"), Some(("host".into(), 0)));
        assert_eq!(parse_address("host:0"), Some(("host".into(), 0)));
        assert_eq!(parse_address("[::1]:65535"), Some(("::1".into(), 65535)));
    }

    #[test]
    fn accept_textures_decodes_numeric_tags_and_save_retains_backup() {
        let dir = std::env::temp_dir().join(format!(
            "sunrise-servers-numeric-backup-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("servers.dat");
        let backup = dir.join("servers.dat_old");

        let entry = |name: &str, ip: &str, accept: Tag| {
            Tag::compound([
                ("name", Tag::String(JavaString::from(name))),
                ("ip", Tag::String(JavaString::from(ip))),
                ("acceptTextures", accept),
            ])
        };
        let raw_root = NamedTag {
            name: JavaString::from(""),
            tag: Tag::compound([(
                "servers",
                Tag::List(vec![
                    entry("IntZero", "int0.example", Tag::Int(0)),
                    entry("Int256", "int256.example", Tag::Int(256)),
                    entry("LongOne", "long1.example", Tag::Long(1)),
                    entry(
                        "FloatNegZero",
                        "fneg0.example",
                        Tag::Float((-0.0_f32).to_bits()),
                    ),
                    entry(
                        "DoubleHalf",
                        "dhalf.example",
                        Tag::Double(0.5_f64.to_bits()),
                    ),
                    entry(
                        "StringIgnored",
                        "str.example",
                        Tag::String(JavaString::from("1")),
                    ),
                ]),
            )]),
        };
        let initial_bytes = nbt::write_named(&raw_root, nbt::Limits::default()).unwrap();
        std::fs::write(&path, &initial_bytes).unwrap();

        let loaded = load(&path);
        assert_eq!(
            loaded
                .iter()
                .map(|e| (e.ip.as_str(), e.accept_textures))
                .collect::<Vec<_>>(),
            vec![
                ("int0.example", Some(false)),
                ("int256.example", Some(true)),
                ("long1.example", Some(true)),
                ("fneg0.example", Some(false)),
                ("dhalf.example", Some(true)),
                ("str.example", None),
            ]
        );

        save(&path, &loaded).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), initial_bytes);
        let (saved_root, _) =
            nbt::read_named(&std::fs::read(&path).unwrap(), nbt::Limits::default()).unwrap();
        let Some(Tag::List(saved_list)) = saved_root.tag.get("servers") else {
            panic!("expected servers list");
        };
        assert_eq!(saved_list[0].get("acceptTextures"), Some(&Tag::Byte(0)));
        assert_eq!(saved_list[1].get("acceptTextures"), Some(&Tag::Byte(1)));
        assert_eq!(saved_list[2].get("acceptTextures"), Some(&Tag::Byte(1)));
        assert_eq!(saved_list[3].get("acceptTextures"), Some(&Tag::Byte(0)));
        assert_eq!(saved_list[4].get("acceptTextures"), Some(&Tag::Byte(1)));
        assert_eq!(saved_list[5].get("acceptTextures"), None);

        // An unreadable file that gets replaced by save_all is kept in servers.dat_old.
        let corrupt = b"corrupt servers.dat";
        std::fs::write(&path, corrupt).unwrap();
        save_all(&path, &[], &[]).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), corrupt);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn disk_decoder_accepts_long_code_of_conduct_and_large_lists_across_second_save() {
        let dir = std::env::temp_dir().join(format!(
            "sunrise-servers-unbounded-coc-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("servers.dat");

        // Vanilla ServerList.load uses NbtAccounter.unlimitedHeap() and CompoundTag.getIntOr()
        // for acceptedCodeOfConduct (NumericTag.intValue). Exceed Limits::default().max_bytes
        // (64 MiB) so a bounded decoder would empty the list on load.
        let shown_entry = Tag::compound([
            ("name", Tag::String(JavaString::from("LargeShown"))),
            ("ip", Tag::String(JavaString::from("shown.example"))),
            ("acceptTextures", Tag::Byte(0)),
            ("acceptedCodeOfConduct", Tag::Long(0x1_0000_002A_i64)),
            ("padding", Tag::ByteArray(vec![0u8; 64 * 1024 * 1024 + 1])),
        ]);
        let hidden_entry = Tag::compound([
            ("name", Tag::String(JavaString::from("LargeHidden"))),
            ("ip", Tag::String(JavaString::from("hidden.example"))),
            ("hidden", Tag::Double(1.9_f64.to_bits())),
            ("acceptedCodeOfConduct", Tag::Double((-3.2_f64).to_bits())),
        ]);
        let raw_root = NamedTag {
            name: JavaString::from(""),
            tag: Tag::compound([("servers", Tag::List(vec![shown_entry, hidden_entry]))]),
        };
        let bytes = nbt::write_named(&raw_root, nbt::Limits::unlimited_heap()).unwrap();
        assert!(nbt::read_named(&bytes, nbt::Limits::default()).is_err());
        std::fs::write(&path, &bytes).unwrap();

        let (shown, hidden) = load_all(&path);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].ip, "shown.example");
        assert_eq!(shown[0].accept_textures, Some(false));
        assert_eq!(shown[0].accepted_code_of_conduct, 0x2A);
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].ip, "hidden.example");
        assert_eq!(hidden[0].accepted_code_of_conduct, -4);

        // Save once, then save a second time: both shown and hidden entries and their
        // acceptedCodeOfConduct values must survive.
        save(&path, &shown).unwrap();
        let (shown2, hidden2) = load_all(&path);
        assert_eq!(shown2, shown);
        assert_eq!(hidden2, hidden);
        save(&path, &shown2).unwrap();
        let (shown3, hidden3) = load_all(&path);
        assert_eq!(shown3, shown);
        assert_eq!(hidden3, hidden);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn save_single_accept_textures_updates_only_the_named_row() {
        let address = "play.example.net:25565";
        let mut shown = vec![
            ServerEntry::new("Home", address),
            ServerEntry::new("Work", address),
        ];
        let mut hidden = vec![ServerEntry::new("Direct", address)];
        assert_eq!(
            find(&shown, &hidden, address, Some("Work")).map(|e| e.name.as_str()),
            Some("Work")
        );
        assert!(save_single_accept_textures(
            &mut shown,
            &mut hidden,
            "Work",
            address,
            Some(true),
        ));
        assert_eq!(shown[0].accept_textures, None);
        assert_eq!(shown[1].accept_textures, Some(true));
        assert_eq!(hidden[0].accept_textures, None);
        assert_eq!(
            find(&shown, &hidden, address, None).map(|e| e.name.as_str()),
            Some("Home")
        );
    }
}
