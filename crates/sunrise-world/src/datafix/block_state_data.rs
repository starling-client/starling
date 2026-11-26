//! `BlockStateData`: the table that maps the 1.12 block states (`blockId << 4 | data`)
//! to 1.13 block states, and the 1.12 `Name` + `Properties` tags to those ids.
//! The table is vanilla's `register` calls, one line each in call order, from the
//! prepared resources (`tables::block_state_data()`, exported from the
//! player's jar). A line is tab separated: the 1.12 id (`blockId << 4 | data`), the
//! 1.13 state's name and properties, then the name and properties of each 1.12 state
//! that maps to it; properties are `key=value` joined by `;`, sorted by key.
//!
//! Vanilla keeps one tag object per `register` call (and per defaulted id) and
//! `ChunkPalettedStorageFix` compares them by identity, so a state is an
//! *instance* here: `register` calls make instances, ids without a call share
//! the instance of the first call for their block.
use std::collections::HashMap;
use std::sync::OnceLock;

use super::dynamic::{as_str, key, string};
use sunrise_core::nbt::{Compound, Tag};

/// A block state: name and properties sorted by key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct State {
    pub name: String,
    pub props: Vec<(String, String)>,
}

impl State {
    pub fn new(name: &str, props: &[(&str, &str)]) -> Self {
        let mut props: Vec<(String, String)> = props
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        props.sort();
        Self {
            name: name.to_owned(),
            props,
        }
    }
    pub fn property(&self, name: &str) -> &str {
        self.props
            .iter()
            .find(|(k, _)| k == name)
            .map_or("", |(_, v)| v)
    }
    /// The NBT tag: `{Name}` or `{Name, Properties}`.
    pub fn to_tag(&self) -> Tag {
        let mut tag = Tag::compound([("Name", string(&self.name))]);
        if !self.props.is_empty() {
            let mut props = Compound::new();
            for (k, v) in &self.props {
                props.insert(key(k), string(v));
            }
            super::dynamic::set(&mut tag, "Properties", Tag::Compound(props));
        }
        tag
    }
    /// A tag read back as a state (`Name` and an optional `Properties` of strings only).
    pub fn from_tag(tag: &Tag) -> Option<Self> {
        let Tag::Compound(c) = tag else {
            return None;
        };
        let name = as_str(c.get(&key("Name")))?;
        let mut props = Vec::new();
        let mut expected = 1;
        if let Some(p) = c.get(&key("Properties")) {
            expected += 1;
            let Tag::Compound(p) = p else {
                return None;
            };
            for (k, v) in p {
                props.push((k.to_string_lossy(), as_str(Some(v))?));
            }
        }
        if c.len() != expected {
            return None;
        }
        props.sort();
        Some(Self { name, props })
    }
}

pub struct Data {
    /// One entry per `register` call.
    pub instances: Vec<State>,
    /// `MAP`: the instance of each id (after the defaulting of `finalizeMaps`).
    map: Vec<Option<u32>>,
    id_by_old: HashMap<State, i32>,
    id_by_old_name: HashMap<String, i32>,
}

fn parse_props(s: &str) -> std::io::Result<Vec<(String, String)>> {
    if s.is_empty() {
        return Ok(Vec::new());
    }
    s.split(';')
        .map(|kv| {
            let (k, v) = kv.split_once('=').ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid block state property",
                )
            })?;
            Ok((k.to_owned(), v.to_owned()))
        })
        .collect()
}

fn parse_data(lines: &[&str]) -> std::io::Result<Data> {
    let mut map: Vec<Option<u32>> = vec![None; 4096];
    let mut defaults: Vec<Option<u32>> = vec![None; 256];
    let mut instances = Vec::new();
    let mut id_by_old = HashMap::new();
    let mut id_by_old_name = HashMap::new();
    let invalid = || {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid block state registration",
        )
    };
    for line in lines {
        let fields: Vec<_> = line.split('\t').collect();
        let [id, name, props, old @ ..] = fields.as_slice() else {
            return Err(invalid());
        };
        let id = id.parse::<usize>().map_err(|_| invalid())?;
        let slot = map.get_mut(id).ok_or_else(invalid)?;
        let (pairs, remainder) = old.as_chunks::<2>();
        if name.is_empty() || !remainder.is_empty() {
            return Err(invalid());
        }
        let instance = instances.len() as u32;
        instances.push(State {
            name: (*name).to_owned(),
            props: parse_props(props)?,
        });
        *slot = Some(instance);
        defaults[id >> 4].get_or_insert(instance);
        for &[name, props] in pairs {
            let old = State {
                name: name.to_owned(),
                props: parse_props(props)?,
            };
            id_by_old_name.entry(old.name.clone()).or_insert(id as i32);
            id_by_old.insert(old, id as i32);
        }
    }
    if map[0].is_none() {
        return Err(invalid());
    }
    for (i, slot) in map.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = defaults[i >> 4];
        }
    }
    Ok(Data {
        instances,
        map,
        id_by_old,
        id_by_old_name,
    })
}

pub(super) fn data() -> super::tables::Result<&'static Data> {
    use super::tables;
    static DATA: OnceLock<tables::Result<Data>> = OnceLock::new();
    let lines = tables::block_state_data()?;
    DATA.get_or_init(|| Ok(parse_data(lines)?))
        .as_ref()
        .map_err(Clone::clone)
}

impl Data {
    /// `getTag(id)` as an instance.
    pub fn instance(&self, id: i32) -> u32 {
        let slot = usize::try_from(id)
            .ok()
            .filter(|i| *i < 4096)
            .and_then(|i| self.map[i]);
        slot.or(self.map[0]).expect("air is registered")
    }
    pub fn state(&self, instance: u32) -> &State {
        &self.instances[instance as usize]
    }
}

/// `BlockStateData.getTag`.
pub(super) fn get_tag(id: i32) -> super::tables::Result<Tag> {
    let d = data()?;
    Ok(d.state(d.instance(id)).to_tag())
}

/// `BlockStateData.upgradeBlockStateTag`.
pub(super) fn upgrade_block_state_tag(old: &Tag) -> super::tables::Result<Tag> {
    let d = data()?;
    Ok(
        match State::from_tag(old).and_then(|s| d.id_by_old.get(&s).copied()) {
            Some(id) if (0..4096).contains(&id) => match d.map[id as usize] {
                Some(i) => d.state(i).to_tag(),
                None => old.clone(),
            },
            _ => old.clone(),
        },
    )
}

/// `BlockStateData.upgradeBlock(String)`.
pub(super) fn upgrade_block_name(old: &str) -> super::tables::Result<String> {
    let d = data()?;
    Ok(match d.id_by_old_name.get(old) {
        Some(&id) if (0..4096).contains(&id) => match d.map[id as usize] {
            Some(i) => d.state(i).name.clone(),
            None => old.to_owned(),
        },
        _ => old.to_owned(),
    })
}

/// `BlockStateData.upgradeBlock(int)`.
pub(super) fn upgrade_block_id(id: i32) -> super::tables::Result<String> {
    let d = data()?;
    Ok(match usize::try_from(id).ok().filter(|i| *i < 4096) {
        Some(i) => match d.map[i] {
            Some(inst) => d.state(inst).name.clone(),
            None => "minecraft:air".to_owned(),
        },
        None => "minecraft:air".to_owned(),
    })
}
