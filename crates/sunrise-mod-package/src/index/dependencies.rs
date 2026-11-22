//! Dependency facts shared by readers, publishers and the install planner.
use super::{Dependency, Kind, Project, Version};
use mod_vocab::Side;
use semver::VersionReq;
use std::collections::HashSet;

impl Dependency {
    pub(super) fn range(&self, owner: &str, version: &str) -> Result<VersionReq, String> {
        VersionReq::parse(&self.version).map_err(|_| {
            format!(
                "{owner} {version} declares {} {}: use a semver range in sunrise-mod.toml and rebuild the index",
                self.id, self.version
            )
        })
    }

    pub(super) fn applies(&self, side: Side) -> bool {
        side == Side::Both || self.side.is_none_or(|s| s == Side::Both || s == side)
    }
}

impl Kind {
    pub(super) fn is_mod(&self) -> bool {
        matches!(self, Self::Mod | Self::NativeMod)
    }
}

/// Validate the rows, not whether their targets still resolve in this index.
pub(super) fn check_shape(p: &Project, v: &Version) -> Result<(), String> {
    let Some(dependencies) = &v.dependencies else {
        return Ok(());
    };
    if !p.kind.is_mod() {
        return Err(format!(
            "{} {}: only mods can declare dependencies",
            p.id, v.version
        ));
    }
    let mut seen = HashSet::new();
    for dep in dependencies {
        if !super::valid_id(&dep.id) || !seen.insert(&dep.id) {
            return Err(format!(
                "{} {} declares {} {}: use a valid, unique mod id in sunrise-mod.toml",
                p.id, v.version, dep.id, dep.version
            ));
        }
        dep.range(&p.id, &v.version)?;
    }
    Ok(())
}

/// Iterative depth-first traversal: index size and graph depth are input, not stack limits.
pub(super) fn required_cycle(edges: &[Vec<usize>]) -> Option<Vec<usize>> {
    enum Mark {
        New,
        Visiting(usize),
        Done,
    }
    let mut marks: Vec<_> = (0..edges.len()).map(|_| Mark::New).collect();
    let mut stack = Vec::new();
    for start in 0..edges.len() {
        if !matches!(marks[start], Mark::New) {
            continue;
        }
        marks[start] = Mark::Visiting(0);
        stack.push((start, edges[start].iter()));
        while let Some((node, next)) = stack.last_mut() {
            match next.next().copied() {
                None => {
                    marks[*node] = Mark::Done;
                    stack.pop();
                }
                Some(to) => match marks[to] {
                    Mark::New => {
                        marks[to] = Mark::Visiting(stack.len());
                        stack.push((to, edges[to].iter()));
                    }
                    Mark::Done => {}
                    Mark::Visiting(at) => {
                        return Some(stack[at..].iter().map(|(i, _)| *i).chain([to]).collect());
                    }
                },
            }
        }
    }
    None
}
