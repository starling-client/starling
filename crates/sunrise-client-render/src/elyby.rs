//! Ely.by's skin system (skinsystem.ely.by): the skins players without a Mojang account keep there
//! under their name, which offline launchers and servers show. Asked by name without signing in,
//! each name once a session on a thread of its own, the answers kept on disk so the next start
//! shows them at once while it asks again; Skin Customization's Ely.by Skins turns it off
//! (`elyby:off` in sunrise-player.txt).
use crate::{offline_name, skins::Textures};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

/// `GET /textures/{nickname}`: the skin's URL and model, 204 for a name without a skin.
const TEXTURES: &str = "https://skinsystem.ely.by/textures/";
const KEY: &str = "elyby";

/// Whether Ely.by's skins show: on unless Skin Customization turned them off.
pub fn enabled(dir: &Path) -> bool {
    offline_name::value(dir, KEY).as_deref() != Some("off")
}

/// Keep Skin Customization's Ely.by Skins in `dir`'s sunrise-player.txt.
pub fn set_enabled(dir: &Path, on: bool) -> std::io::Result<()> {
    offline_name::set_values(dir, &[(KEY, Some(if on { "on" } else { "off" }))])
}

/// Whether `name` can be a player's (1 to 16 letters, digits or `_`): an NPC's styled name is
/// never asked for.
pub fn is_name(name: &str) -> bool {
    (1..=16).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The skin of `/textures/{nickname}`'s answer, `{"SKIN": {"url": ..., "metadata": {"model":
/// "slim"}}}`; None when it names none. A cape is left out: one that failed to download would
/// fail the skin with it (`skins::lookup`).
pub fn parse(json: &str) -> Option<Textures> {
    let answer: serde_json::Value = serde_json::from_str(json).ok()?;
    let skin = &answer["SKIN"];
    let url = skin["url"].as_str()?;
    Some(Textures {
        skin: Some(url.to_owned()),
        slim: skin["metadata"]["model"].as_str() == Some("slim"),
        ..Textures::default()
    })
}

/// The most names kept on disk: a session on a busy server does not grow the file without end.
const KEPT_NAMES: usize = 512;

/// What Ely.by answered for a name: its skin, or None for a name without one.
type Answer = Option<Textures>;

/// Names asked of Ely.by and what it answered, this session and the last ones.
pub struct Lookup {
    /// This session's answers, by the name in lower case (Ely.by tells names apart without case).
    answers: HashMap<String, Answer>,
    /// Asked this session, not answered yet.
    asked: HashSet<String>,
    /// The answers kept on disk (`keep_in`), shown until this session's come.
    kept: HashMap<String, Answer>,
    file: Option<PathBuf>,
    /// None when Ely.by gave no answer (offline, a timeout): what was kept stays.
    send: Sender<(String, Option<Answer>)>,
    done: Receiver<(String, Option<Answer>)>,
}

impl Default for Lookup {
    fn default() -> Self {
        let (send, done) = channel();
        Self {
            answers: HashMap::new(),
            asked: HashSet::new(),
            kept: HashMap::new(),
            file: None,
            send,
            done,
        }
    }
}

impl Lookup {
    /// Keep the answers in `file`, and read the ones it holds.
    pub fn keep_in(&mut self, file: PathBuf) {
        self.kept = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|v| v.as_object().cloned())
            .map(|names| {
                names
                    .into_iter()
                    .map(|(name, answer)| (name, answer_of(&answer)))
                    .collect()
            })
            .unwrap_or_default();
        self.file = Some(file);
    }

    /// `name`'s skin on Ely.by: this session's answer, else the one kept from the last (None
    /// before either, and for a name without one); asked the first time.
    pub fn textures(&mut self, name: &str) -> Option<Textures> {
        self.receive();
        if !is_name(name) {
            return None;
        }
        let key = name.to_ascii_lowercase();
        if let Some(answer) = self.answers.get(&key) {
            return answer.clone();
        }
        if self.asked.insert(key.clone()) {
            let send = self.send.clone();
            let ask = key.clone();
            let asked = std::thread::Builder::new()
                .name("ely.by".into())
                .spawn(move || {
                    let answer = fetch(&ask);
                    let _ = send.send((ask, answer));
                });
            if let Err(e) = asked {
                log::warn!("Sunrise: Ely.by skin: {e}");
            }
        }
        self.kept.get(&key).cloned().flatten()
    }

    /// Whether `name` waits for Ely.by with nothing kept to show meanwhile.
    pub fn waiting(&mut self, name: &str) -> bool {
        self.receive();
        let key = name.to_ascii_lowercase();
        is_name(name)
            && !self.answers.contains_key(&key)
            && !self.kept.contains_key(&key)
            && self.asked.contains(&key)
    }

    /// The answers that came, kept on disk when they changed what was kept.
    fn receive(&mut self) {
        let mut changed = false;
        while let Ok((key, answer)) = self.done.try_recv() {
            // No answer: what was kept stands for this session.
            let answer = match answer {
                Some(answer) => answer,
                None => self.kept.get(&key).cloned().flatten(),
            };
            if self.kept.get(&key) != Some(&answer) {
                if self.kept.len() >= KEPT_NAMES
                    && !self.kept.contains_key(&key)
                    && let Some(old) = self.kept.keys().next().cloned()
                {
                    self.kept.remove(&old);
                }
                self.kept.insert(key.clone(), answer.clone());
                changed = true;
            }
            self.answers.insert(key, answer);
        }
        if changed && let Some(file) = &self.file {
            let names: serde_json::Map<String, serde_json::Value> = self
                .kept
                .iter()
                .map(|(name, answer)| (name.clone(), value_of(answer)))
                .collect();
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(file, serde_json::Value::Object(names).to_string()) {
                log::warn!("Sunrise: could not keep Ely.by's skins: {e}");
            }
        }
    }
}

/// An answer as the file keeps it: `{"skin": url, "slim": bool}`, or null.
fn value_of(answer: &Answer) -> serde_json::Value {
    match answer {
        Some(t) => serde_json::json!({"skin": t.skin, "slim": t.slim}),
        None => serde_json::Value::Null,
    }
}

fn answer_of(value: &serde_json::Value) -> Answer {
    Some(Textures {
        skin: Some(value["skin"].as_str()?.to_owned()),
        slim: value["slim"].as_bool() == Some(true),
        ..Textures::default()
    })
}

/// Ely.by's answer for `name`: Some(its skin, or None for a name without one), None when it gave
/// no answer.
fn fetch(name: &str) -> Option<Answer> {
    use std::io::Read;
    let response = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .get(&format!("{TEXTURES}{name}"))
        .call()
        .ok()?;
    if response.status() != 200 {
        // 204: a name without a skin.
        return (response.status() == 204).then_some(None);
    }
    let mut body = String::new();
    response
        .into_reader()
        .take(64 << 10)
        .read_to_string(&mut body)
        .ok()?;
    Some(parse(&body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skin_and_its_model_from_the_answer() {
        let wide = r#"{"SKIN":{"url":"http://ely.by/storage/skins/69c6740d2993e5d6f6a7fc92420efc29.png"}}"#;
        let t = parse(wide).unwrap();
        assert_eq!(
            t.skin.as_deref(),
            Some("http://ely.by/storage/skins/69c6740d2993e5d6f6a7fc92420efc29.png")
        );
        assert!(!t.slim && !t.signed && t.cape.is_none());
        let slim = r#"{"SKIN":{"url":"http://ely.by/storage/skins/a.png","metadata":{"model":"slim"}},"CAPE":{"url":"http://ely.by/storage/cloaks/b.png"}}"#;
        let t = parse(slim).unwrap();
        assert!(t.slim);
        assert!(t.cape.is_none());
        assert!(parse("").is_none());
        assert!(parse(r#"{"CAPE":{"url":"http://ely.by/storage/cloaks/b.png"}}"#).is_none());
    }

    #[test]
    fn only_player_names_are_asked() {
        assert!(is_name("ErickSkrauch"));
        assert!(is_name("a_1"));
        assert!(!is_name(""));
        assert!(!is_name("§6Shopkeeper"));
        assert!(!is_name("seventeen_letters"));
        assert!(!is_name("two words"));
        let mut lookup = Lookup::default();
        assert!(lookup.textures("§cNPC").is_none());
        assert!(lookup.answers.is_empty());
    }

    #[test]
    fn turned_off_and_on_in_the_player_file() {
        let dir = std::env::temp_dir().join(format!("sunrise-elyby-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        assert!(enabled(&dir));
        set_enabled(&dir, false).unwrap();
        assert!(!enabled(&dir));
        set_enabled(&dir, true).unwrap();
        assert!(enabled(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn answers_kept_on_disk_show_at_the_next_start() {
        let dir = std::env::temp_dir().join(format!("sunrise-elyby-kept-{}", std::process::id()));
        let file = dir.join("elyby.json");
        let _ = std::fs::remove_dir_all(&dir);
        let skin = Textures {
            skin: Some("http://ely.by/storage/skins/a.png".into()),
            slim: true,
            ..Textures::default()
        };
        let mut lookup = Lookup::default();
        lookup.keep_in(file.clone());
        // As the asking thread would answer, without asking.
        lookup
            .asked
            .extend(["steve".to_owned(), "nobody".to_owned()]);
        lookup
            .send
            .send(("steve".into(), Some(Some(skin.clone()))))
            .unwrap();
        lookup.send.send(("nobody".into(), Some(None))).unwrap();
        assert_eq!(lookup.textures("Steve"), Some(skin.clone()));
        assert_eq!(lookup.textures("nobody"), None);
        let mut next = Lookup::default();
        next.keep_in(file);
        next.asked
            .extend(["steve".to_owned(), "nobody".to_owned(), "new".to_owned()]);
        assert_eq!(next.textures("STEVE"), Some(skin));
        assert!(!next.waiting("steve") && !next.waiting("nobody"));
        assert!(next.waiting("new"));
        // No answer this time: what was kept stands.
        next.send.send(("steve".into(), None)).unwrap();
        assert!(next.textures("steve").is_some_and(|t| t.slim));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "asks skinsystem.ely.by"]
    fn ely_by_answers_a_known_name() {
        let t = fetch("ErickSkrauch").expect("an answer").expect("a skin");
        assert!(t.skin.unwrap().contains("ely.by"));
        assert_eq!(fetch("zzzznosuchname123"), Some(None));
    }
}
