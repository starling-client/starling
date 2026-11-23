//! The offline player name, when neither `--name`, `SUNRISE_PLAYER_NAME` nor a
//! signed-in account gives one. Vanilla's client defaults `--username` to
//! `"Player" + System.currentTimeMillis() % 1000` (net.minecraft.client.main.Main)
//! and the official launcher always passes the account's name, so two players
//! rarely share one. Sunrise used plain `Player` for everyone, which is also
//! the LAN host's own profile: every guest got the host's offline UUID and was
//! refused with "You logged in from another location". The generated name is
//! kept in `sunrise-player.txt` next to `options.txt` so the same player gets
//! the same offline UUID (and inventory on a friend's world) every time.
//!
//! The player may choose the name instead (Options > Skin Customization): the file then marks it
//! chosen, and worlds open with it even while an account is signed in. The file also keeps the
//! player's own skin (`own_skin`). One `key:value` per line.
use std::{fs, path::Path};

pub const FILE: &str = "sunrise-player.txt";
const NAME: &str = "name";
/// Marks the name as the player's choice, not a generated one.
const CHOSEN: &str = "chosen";

/// StringUtil.isValidPlayerName, plus not empty (the login screen sends at least one character):
/// the LAN host's check too (sunrise-client's `lan::conn::valid_name`).
pub fn is_valid(name: &str) -> bool {
    (1..=16).contains(&name.len()) && name.chars().all(|c| c as u32 > 32 && (c as u32) < 127)
}

/// A name the player may choose: 3 to 16 of `A-Z`, `a-z`, `0-9` and `_`, the names Mojang gives
/// profiles, which every server takes.
pub fn is_valid_choice(name: &str) -> bool {
    (3..=16).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Keep `name` as the player's chosen name; refused, the file as it was, unless
/// [`is_valid_choice`].
pub fn choose(dir: &Path, name: &str) -> Result<(), String> {
    if !is_valid_choice(name) {
        return Err(format!(
            "\"{name}\" is not a name: 3 to 16 letters, digits or _"
        ));
    }
    set_values(dir, &[(NAME, Some(name)), (CHOSEN, Some("true"))])
        .map_err(|e| format!("could not keep the name: {e}"))
}

/// The name the player chose, if they chose one.
pub fn chosen(dir: &Path) -> Option<String> {
    if value(dir, CHOSEN).as_deref() != Some("true") {
        return None;
    }
    value(dir, NAME).filter(|name| is_valid(name))
}

/// The name single player's worlds open with: the chosen one, else the signed-in account's, else
/// the kept offline name.
pub fn for_worlds(dir: &Path) -> String {
    match crate::auth::current() {
        Some(account) => chosen(dir).unwrap_or_else(|| account.name.clone()),
        None => load_or_create(dir),
    }
}

/// `key`'s value in `dir`'s sunrise-player.txt.
pub fn value(dir: &Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(dir.join(FILE)).ok()?;
    text.lines().find_map(|line| {
        let (k, v) = line.trim_end_matches('\r').split_once(':')?;
        (k == key).then(|| v.to_owned())
    })
}

/// Set these keys in `dir`'s sunrise-player.txt (None removes one), each in its line's place or
/// after the others, keeping every other line.
pub fn set_values(dir: &Path, values: &[(&str, Option<&str>)]) -> std::io::Result<()> {
    let path = dir.join(FILE);
    let text = fs::read_to_string(&path).unwrap_or_default();
    let mut done = vec![false; values.len()];
    let mut out = String::new();
    fn put(out: &mut String, key: &str, value: &str) {
        out.push_str(key);
        out.push(':');
        out.push_str(value);
        out.push('\n');
    }
    for line in text.lines().map(|l| l.trim_end_matches('\r')) {
        let at = line
            .split_once(':')
            .and_then(|(k, _)| values.iter().position(|(key, _)| *key == k));
        match at {
            Some(i) => {
                if let (false, Some(value)) = (done[i], values[i].1) {
                    put(&mut out, values[i].0, value);
                }
                done[i] = true;
            }
            None if line.is_empty() => {}
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    for (i, (key, value)) in values.iter().enumerate() {
        if let (false, Some(value)) = (done[i], value) {
            put(&mut out, key, value);
        }
    }
    fs::write(&path, out)
}

/// `Player` and five digits (vanilla's default has three: one shared name in a
/// thousand pairs is too many for a game that keeps it).
pub fn generate(seed: u64) -> String {
    format!("Player{:05}", seed % 100_000)
}

/// The name kept in `dir`, created on first use. A file that can't be written
/// still yields a name for this run.
pub fn load_or_create(dir: &Path) -> String {
    if let Some(name) = value(dir, NAME).filter(|name| is_valid(name)) {
        return name;
    }
    let name = generate(seed());
    if let Err(e) = set_values(dir, &[(NAME, Some(&name)), (CHOSEN, None)]) {
        log::warn!(
            "Sunrise: could not keep the player name in {}: {e}",
            dir.join(FILE).display()
        );
    }
    name
}

fn seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    // Two players starting the game in the same millisecond still differ.
    let mut x = nanos ^ u64::from(std::process::id()).rotate_left(32);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^ (x >> 33)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "sunrise-offline-name-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn generated_names_are_valid_and_never_the_hosts_profile() {
        for seed in [0, 7, 99_999, 100_000, u64::MAX] {
            let name = generate(seed);
            assert!(is_valid(&name), "{name}");
            // The LAN host's own profile (game/chat.rs PLAYER_NAME).
            assert_ne!(name, "Player");
            assert_eq!(name.len(), 11);
        }
        assert_eq!(generate(42), "Player00042");
    }

    #[test]
    fn the_name_is_kept_and_reused() {
        let d = dir("kept");
        let first = load_or_create(&d);
        assert_eq!(
            fs::read_to_string(d.join(FILE)).unwrap(),
            format!("name:{first}\n")
        );
        assert_eq!(load_or_create(&d), first);
        // An edited name is used as it is.
        fs::write(d.join(FILE), "name:Alex_2\r\n").unwrap();
        assert_eq!(load_or_create(&d), "Alex_2");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn invalid_names_are_replaced() {
        for text in [
            "",
            "name:",
            "name:has space",
            "name:ThisNameIsTooLong17",
            "name:été",
        ] {
            let d = dir("invalid");
            fs::write(d.join(FILE), text).unwrap();
            let name = load_or_create(&d);
            assert!(
                name.starts_with("Player") && is_valid(&name),
                "{text:?} -> {name}"
            );
            assert_eq!(load_or_create(&d), name);
            let _ = fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn a_chosen_name_is_three_to_sixteen_letters_digits_or_underscores() {
        for good in [
            "Abc",
            "Notch",
            "jeb_",
            "Player00042",
            "A_b_C_1234567890",
            "___",
        ] {
            assert!(is_valid_choice(good), "{good}");
        }
        for bad in [
            "",
            "ab",
            "ThisNameIsTooLong",
            "has space",
            "dash-name",
            "été",
            "Ünder",
            "dot.name",
            "tab\tname",
        ] {
            assert!(!is_valid_choice(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_chosen_name_is_kept_beside_the_other_lines() {
        let d = dir("chosen");
        let generated = load_or_create(&d);
        assert_eq!(chosen(&d), None);
        fs::write(
            d.join(FILE),
            format!("name:{generated}\r\nskin:alex.png\nmodel:slim\n"),
        )
        .unwrap();
        assert!(choose(&d, "no").is_err());
        assert!(choose(&d, "bad name").is_err());
        assert_eq!(chosen(&d), None);
        assert_eq!(load_or_create(&d), generated);
        choose(&d, "Steve_2").unwrap();
        assert_eq!(
            fs::read_to_string(d.join(FILE)).unwrap(),
            "name:Steve_2\nskin:alex.png\nmodel:slim\nchosen:true\n"
        );
        assert_eq!(chosen(&d).as_deref(), Some("Steve_2"));
        assert_eq!(load_or_create(&d), "Steve_2");
        // A name replaced by a generated one is no longer the player's choice; the rest stays.
        set_values(&d, &[(NAME, Some("bad name"))]).unwrap();
        let again = load_or_create(&d);
        assert!(again.starts_with("Player"), "{again}");
        assert_eq!(chosen(&d), None);
        assert_eq!(value(&d, "skin").as_deref(), Some("alex.png"));
        assert_eq!(value(&d, "model").as_deref(), Some("slim"));
        let _ = fs::remove_dir_all(&d);
    }
}
