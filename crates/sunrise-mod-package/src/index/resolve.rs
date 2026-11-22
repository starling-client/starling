//! The registry's install and removal rules. Planning does not download or write any files.
use super::{Dependency, Index, Kind, Project, Version, dependencies::required_cycle};
use crate::manifest::parse_version;
use mod_vocab::Side;
use semver::{Comparator, Op, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_STATES: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModVersion {
    pub id: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledMod {
    pub id: String,
    pub version: String,
    /// Only an exact hash match identifies the registry artifact. Otherwise this is a local copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

impl InstalledMod {
    fn is_file(&self, version: &Version) -> bool {
        self.version == version.version
            && self
                .sha256
                .as_ref()
                .is_some_and(|h| h.eq_ignore_ascii_case(&version.file.sha256))
    }
}

type Present<'a> = BTreeMap<&'a str, &'a InstalledMod>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    /// None keeps an eligible installed version or asks for the newest one that works.
    pub version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RequestResult {
    Installable { version: String },
    AlreadyInstalled { version: String },
    Refused { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    pub id: String,
    pub from: String,
    pub to: String,
    /// The replacement has lower SemVer precedence than the installed version.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub downgrade: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionalDependency {
    pub from: ModVersion,
    pub dependency: Dependency,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InstallPlan {
    pub requested: BTreeMap<String, RequestResult>,
    pub install: Vec<ModVersion>,
    pub kept: Vec<ModVersion>,
    /// Already on disk and in range, but the registry has no manifest facts for this version.
    pub kept_unindexed: Vec<ModVersion>,
    pub update: Vec<Update>,
    pub optional: Vec<OptionalDependency>,
    /// Dependency id to requested ids that need it, directly or transitively.
    pub required_by: BTreeMap<String, Vec<String>>,
}

/// Resolve requested mods and their required dependencies together. If the batch fails, plan
/// what can install independently. Every plan list is sorted by id (optional declarations by
/// declaring id then dependency id).
///
/// `native_fits` is the caller's per-file SDK fit decision. The index only knows what build a
/// native mod was made for, not whether its used API surface is unchanged on another build.
/// `side` is the installation (client or dedicated server); `Both` considers all declarations.
/// The caller checks takedowns by hash for installed files: only it has their actual bytes.
/// A local copy with no matching index hash has no dependency metadata to walk. This planner
/// checks its version range and any project-wide takedown; the loader checks its manifest.
pub fn install(
    index: &Index,
    requests: &[Request],
    installed: &[InstalledMod],
    game: &str,
    side: Side,
    native_fits: impl Fn(&Project, &Version) -> bool,
) -> InstallPlan {
    let mut plan = InstallPlan::default();
    let mut roots = Roots::new();
    for r in requests {
        roots.entry(&r.id).or_default().extend(r.version.as_deref());
    }
    if roots.is_empty() {
        return plan;
    }
    for pins in roots.values_mut() {
        pins.sort_unstable();
        pins.dedup();
    }
    let mut present = BTreeMap::new();
    for m in installed {
        if let Some(previous) = present.insert(m.id.as_str(), m)
            && previous != m
        {
            let reason = format!(
                "{} has conflicting installed files ({} and {}): keep only one",
                m.id, previous.version, m.version
            );
            refuse(&mut plan, &roots, &reason);
            return plan;
        }
    }
    let catalog = Catalog::new(index, game, side, native_fits, &present);
    let installed_choices: Selected<'_> = present
        .keys()
        .filter_map(|&id| Some((id, catalog.mods.get(id)?.iter().find(|c| c.installed)?)))
        .collect();

    let selected = match catalog.solve(&roots, &installed_choices) {
        Ok(selected) => selected,
        Err(reason) if roots.len() == 1 => {
            refuse(&mut plan, &roots, &reason);
            return plan;
        }
        Err(_) => {
            let selected = catalog.solve_independent(&roots, &installed_choices, &mut plan);
            roots.retain(|id, _| !plan.requested.contains_key(*id));
            selected
        }
    };
    let mut needed_by = BTreeMap::<_, BTreeSet<_>>::new();
    for &root in roots.keys() {
        let mut todo = vec![root];
        let mut visited = BTreeSet::new();
        while let Some(id) = todo.pop() {
            if !visited.insert(id) {
                continue;
            }
            if id != root {
                needed_by
                    .entry(id.to_owned())
                    .or_default()
                    .insert(root.to_owned());
            }
            todo.extend(selected[id].required.iter().map(|d| d.row.id.as_str()));
        }
    }
    for (id, candidate) in selected {
        let version = candidate.version;
        let row = ModVersion {
            id: id.into(),
            version: version.into(),
        };
        let outcome = if candidate.installed {
            if candidate.unindexed {
                plan.kept_unindexed.push(row.clone());
            } else {
                plan.kept.push(row.clone());
            }
            RequestResult::AlreadyInstalled {
                version: version.into(),
            }
        } else {
            if let Some(old) = installed_choices.get(id) {
                let downgrade = candidate
                    .number
                    .as_ref()
                    .zip(old.number.as_ref())
                    .is_some_and(|(next, previous)| next.cmp_precedence(previous).is_lt());
                plan.update.push(Update {
                    id: id.into(),
                    from: old.version.into(),
                    to: version.into(),
                    downgrade,
                });
            } else {
                plan.install.push(row.clone());
            }
            RequestResult::Installable {
                version: version.into(),
            }
        };
        if roots.contains_key(id) {
            plan.requested.insert(id.into(), outcome);
        }
        plan.optional
            .extend(candidate.optional.iter().map(|dep| OptionalDependency {
                from: row.clone(),
                dependency: dep.row.clone(),
            }));
    }
    plan.optional
        .sort_by(|a, b| (&a.from.id, &a.dependency.id).cmp(&(&b.from.id, &b.dependency.id)));
    plan.required_by = needed_by
        .into_iter()
        .map(|(id, by)| (id, by.into_iter().collect()))
        .collect();
    plan
}

/// List installed mods that directly or transitively require `id`, from the indexed metadata
/// for matching installed artifacts. An optional or out-of-side dependency is not a blocker.
pub fn removal_dependents(
    index: &Index,
    installed: &[InstalledMod],
    id: &str,
    side: Side,
) -> Result<Vec<ModVersion>, String> {
    let mut reverse = BTreeMap::<&str, Vec<&InstalledMod>>::new();
    for m in installed {
        let Some(version) = index
            .project(&m.id)
            .filter(|p| p.kind.is_mod())
            .and_then(|p| p.versions.iter().find(|v| m.is_file(v)))
        else {
            continue;
        };
        for dep in version
            .dependencies
            .iter()
            .flatten()
            .filter(|d| !d.optional && d.applies(side))
        {
            dep.range(&m.id, &m.version)?;
            reverse.entry(&dep.id).or_default().push(m);
        }
    }
    let mut seen = BTreeSet::from([id]);
    let mut queue = vec![id];
    let mut dependents = Vec::new();
    while let Some(id) = queue.pop() {
        for m in reverse.get(id).into_iter().flatten() {
            if seen.insert(&m.id) {
                queue.push(&m.id);
                dependents.push(ModVersion {
                    id: m.id.clone(),
                    version: m.version.clone(),
                });
            }
        }
    }
    dependents.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(dependents)
}

#[derive(PartialEq, Eq)]
enum Fit {
    Yes,
    Revoked,
    WrongGame,
    Unsupported(String),
    WrongSunrise,
}

struct Range<'a> {
    row: &'a Dependency,
    parsed: Result<VersionReq, String>,
}

struct Candidate<'a> {
    version: &'a str,
    installed: bool,
    unindexed: bool,
    number: Option<semver::Version>,
    fit: Fit,
    required: Vec<Range<'a>>,
    optional: Vec<Range<'a>>,
}

struct Catalog<'a> {
    game: &'a str,
    mods: BTreeMap<&'a str, Vec<Candidate<'a>>>,
}

type Roots<'a> = BTreeMap<&'a str, Vec<&'a str>>;
type Selected<'a> = BTreeMap<&'a str, &'a Candidate<'a>>;

struct SolveFailure<'a> {
    reason: String,
    // Installed artifacts whose replacement may make an explored assignment viable.
    replacements: BTreeSet<&'a str>,
}

struct Group<'a> {
    roots: Roots<'a>,
    selected: Selected<'a>,
    // A selection ignoring fixed installed ranges is only a hint for grouping, never a plan.
    blocked: Option<SolveFailure<'a>>,
}

impl<'a> Group<'a> {
    fn validated(roots: Roots<'a>, selected: Selected<'a>) -> Self {
        Self {
            roots,
            selected,
            blocked: None,
        }
    }
}

fn refuse(plan: &mut InstallPlan, roots: &Roots<'_>, reason: &str) {
    for &id in roots.keys() {
        plan.requested.insert(
            id.to_owned(),
            RequestResult::Refused {
                reason: reason.to_owned(),
            },
        );
    }
}

fn interacting_groups<'a>(groups: Vec<Group<'a>>, installed: &Selected<'a>) -> Vec<Vec<Group<'a>>> {
    fn representative(parents: &mut [usize], mut i: usize) -> usize {
        while parents[i] != i {
            parents[i] = parents[parents[i]];
            i = parents[i];
        }
        i
    }

    // Shared ids join all their groups to one representative, not to every pair of groups.
    let owners: BTreeMap<_, _> = groups
        .iter()
        .enumerate()
        .flat_map(|(i, group)| group.selected.keys().map(move |&id| (id, i)))
        .collect();
    let mut parents: Vec<_> = (0..groups.len()).collect();
    for (i, group) in groups.iter().enumerate() {
        // Another selection can share a mod, activate an optional range, replace an installed
        // owner, or satisfy a guard that blocked an alternative version of this request.
        let targets = group.selected.iter().flat_map(|(&id, candidate)| {
            std::iter::once(id)
                .chain(candidate.optional.iter().map(|r| r.row.id.as_str()))
                .chain(
                    installed
                        .get(id)
                        .into_iter()
                        .flat_map(|old| old.required.iter().chain(&old.optional))
                        .map(|r| r.row.id.as_str()),
                )
        });
        let replacements = group
            .blocked
            .iter()
            .flat_map(|f| f.replacements.iter().copied());
        for target in targets.chain(replacements) {
            if let Some(&j) = owners.get(target)
                && i != j
            {
                let left = representative(&mut parents, i);
                let right = representative(&mut parents, j);
                parents[left.max(right)] = left.min(right);
            }
        }
    }
    let mut components = BTreeMap::<_, Vec<_>>::new();
    for (i, group) in groups.into_iter().enumerate() {
        components
            .entry(representative(&mut parents, i))
            .or_default()
            .push(group);
    }
    components.into_values().collect()
}

struct Need<'a> {
    pins: &'a [&'a str],
    ranges: Vec<(&'a str, &'a Range<'a>)>,
}

impl Need<'_> {
    fn accepts(&self, candidate: &Candidate<'_>) -> bool {
        self.pins.iter().all(|v| *v == candidate.version)
            && self.ranges.iter().all(|(_, r)| {
                r.parsed
                    .as_ref()
                    .is_ok_and(|req| candidate.number.as_ref().is_some_and(|n| req.matches(n)))
            })
    }

    fn describe(&self) -> String {
        self.pins
            .iter()
            .map(|v| format!("requested {v}"))
            .chain(
                self.ranges
                    .iter()
                    .map(|(owner, r)| format!("{owner} needs {}", r.row.version)),
            )
            .collect::<Vec<_>>()
            .join("; ")
    }
}

impl<'a> Catalog<'a> {
    fn new(
        index: &'a Index,
        game: &'a str,
        side: Side,
        native_fits: impl Fn(&Project, &Version) -> bool,
        present: &Present<'a>,
    ) -> Self {
        let mut mods: BTreeMap<_, Vec<_>> = index
            .projects
            .iter()
            .filter(|p| p.kind.is_mod())
            .map(|p| {
                let mut versions: Vec<_> = p
                    .versions
                    .iter()
                    .map(|v| {
                        let mut required = Vec::new();
                        let mut optional = Vec::new();
                        for dep in v.dependencies.iter().flatten().filter(|d| d.applies(side)) {
                            let parsed = dep.range(&p.id, &v.version);
                            if dep.optional {
                                optional.push(Range { row: dep, parsed });
                            } else {
                                required.push(Range { row: dep, parsed });
                            }
                        }
                        Candidate {
                            version: &v.version,
                            installed: present.get(p.id.as_str()).is_some_and(|m| m.is_file(v)),
                            unindexed: false,
                            number: parse_version(&v.version).ok(),
                            fit: if index.revoked(&p.id, &v.file.sha256).is_some() {
                                Fit::Revoked
                            } else if v.game != game {
                                Fit::WrongGame
                            } else if let Some(reason) = v.capabilities.unsupported() {
                                Fit::Unsupported(reason)
                            } else if p.kind == Kind::NativeMod && !native_fits(p, v) {
                                Fit::WrongSunrise
                            } else {
                                Fit::Yes
                            },
                            required,
                            optional,
                        }
                    })
                    .collect();
                // Equal SemVer precedence keeps the index's newest-first order.
                versions.sort_by(|a, b| match (&a.number, &b.number) {
                    (Some(a), Some(b)) => b.cmp_precedence(a),
                    _ => b.number.is_some().cmp(&a.number.is_some()),
                });
                (p.id.as_str(), versions)
            })
            .collect();
        for (&id, &row) in present {
            let version = &row.version;
            let candidates = mods.entry(id).or_default();
            if !candidates.iter().any(|c| c.installed) {
                candidates.push(Candidate {
                    version,
                    installed: true,
                    unindexed: true,
                    number: parse_version(version).ok(),
                    fit: if index.revoked(id, "").is_some() {
                        Fit::Revoked
                    } else {
                        Fit::Yes
                    },
                    required: Vec::new(),
                    optional: Vec::new(),
                });
            }
        }
        Self { game, mods }
    }

    fn choices(&self, id: &str, need: &Need<'_>) -> Result<Vec<&Candidate<'a>>, String> {
        let describe = || {
            let needs = need.describe();
            if needs.is_empty() {
                id.to_owned()
            } else {
                format!("{id} ({needs})")
            }
        };
        let Some(versions) = self.mods.get(id) else {
            return Err(format!(
                "{}: not listed as a mod in the registry",
                describe()
            ));
        };
        let mut matches = versions.iter().filter(|c| need.accepts(c)).peekable();
        if matches.peek().is_none() {
            if need.pins.is_empty()
                && !need.ranges.is_empty()
                && let Some(pre) = versions
                    .iter()
                    .filter(|c| !c.unindexed)
                    .filter_map(|c| c.number.as_ref())
                    .find(|n| {
                        !n.pre.is_empty()
                            && need.ranges.iter().all(|(_, r)| {
                                r.parsed.as_ref().is_ok_and(|range| {
                                    range.matches(n) || excluded_as_prerelease(range, n)
                                })
                            })
                    })
            {
                return Err(format!(
                    "{}: no listed version satisfies all requirements; use a range that names the prerelease, such as ={pre}",
                    describe()
                ));
            }
            return Err(format!(
                "{}: no listed version satisfies all requirements",
                describe()
            ));
        }
        let mut unrevoked = matches.filter(|c| c.fit != Fit::Revoked).peekable();
        if unrevoked.peek().is_none() {
            return Err(format!(
                "{}: every matching version was taken down",
                describe()
            ));
        }
        let mut game_fits = unrevoked.filter(|c| c.fit != Fit::WrongGame).peekable();
        let Some(first) = game_fits.peek().copied() else {
            return Err(format!(
                "{}: no matching version fits this instance (Minecraft {})",
                describe(),
                self.game
            ));
        };
        let mut supported = game_fits
            .filter(|c| !matches!(c.fit, Fit::Unsupported(_)))
            .peekable();
        if supported.peek().is_none()
            && let Fit::Unsupported(reason) = &first.fit
        {
            return Err(format!("{} {}: {reason}", describe(), first.version));
        }
        let fits: Vec<_> = supported.filter(|c| c.fit == Fit::Yes).collect();
        if fits.is_empty() {
            return Err(format!(
                "{}: no matching native version fits this Sunrise build",
                describe()
            ));
        }
        Ok(fits)
    }

    fn solve_independent<'s>(
        &'s self,
        roots: &Roots<'s>,
        installed: &Selected<'s>,
        plan: &mut InstallPlan,
    ) -> Selected<'s> {
        // Group guarded singleton selections by their interactions. For a blocked request,
        // a guard-free selection and blockers seen during guarded backtracking only establish
        // links to peers. If a mixed group fails, retry without some of its blocked requests,
        // keeping validated ones. Only a guarded success becomes part of the plan.
        let mut groups = Vec::new();
        for (&id, pins) in roots {
            let roots = Roots::from([(id, pins.clone())]);
            let mut budget = MAX_STATES;
            match self.solve_with_budget(&roots, installed, &mut budget) {
                Ok(selected) => groups.push(Group::validated(roots, selected)),
                Err(failure) => {
                    if !installed.is_empty()
                        && let Ok(selected) = self.solve(&roots, &Selected::new())
                    {
                        groups.push(Group {
                            roots,
                            selected,
                            blocked: Some(failure),
                        });
                    } else {
                        refuse(plan, &roots, &failure.reason);
                    }
                }
            }
        }
        loop {
            let count = groups.len();
            let components = interacting_groups(std::mem::take(&mut groups), installed);
            for component in components {
                let mut limit = None;
                if component.len() > 1 {
                    let roots = component.iter().flat_map(|g| g.roots.clone()).collect();
                    let mut budget = MAX_STATES;
                    match self.solve_with_budget(&roots, installed, &mut budget) {
                        Ok(selected) => {
                            groups.push(Group::validated(roots, selected));
                            continue;
                        }
                        Err(failure) if component.iter().all(|g| g.blocked.is_none()) => {
                            let ids = roots.keys().copied().collect::<Vec<_>>().join(", ");
                            refuse(
                                plan,
                                &roots,
                                &format!("{ids} cannot be installed together: {}", failure.reason),
                            );
                            continue;
                        }
                        Err(_) => {
                            match self.recover_component(&component, installed, &mut budget, plan) {
                                Ok(Some(group)) => {
                                    groups.push(group);
                                    continue;
                                }
                                Ok(None) => {}
                                Err(reason) => limit = Some(reason),
                            }
                        }
                    }
                }
                // A speculative selection must not cause a viable request to be refused.
                for group in component {
                    if let Some(failure) = group.blocked {
                        refuse(plan, &group.roots, limit.unwrap_or(&failure.reason));
                    } else {
                        groups.push(group);
                    }
                }
            }
            // Merging can change selections and create new edges. Every merge/refusal removes
            // a group; once the count stops decreasing, all survivors are validated and disjoint.
            if groups.len() == count {
                return groups
                    .into_iter()
                    .flat_map(|group| group.selected)
                    .collect();
            }
        }
    }

    fn recover_component<'s>(
        &'s self,
        component: &[Group<'s>],
        installed: &Selected<'s>,
        budget: &mut usize,
        plan: &mut InstallPlan,
    ) -> Result<Option<Group<'s>>, &'static str> {
        let blocked: Vec<_> = component
            .iter()
            .enumerate()
            .filter_map(|(i, g)| g.blocked.as_ref().map(|failure| (i, failure)))
            .collect();
        // Try the largest subsets first. Enumerate only omissions of blocked requests; if
        // none can join the group, the caller regroups the validated survivors on their own.
        'subsets: for count in 1..blocked.len() {
            let mut omitted: Vec<_> = (0..count).collect();
            loop {
                if *budget == 0 {
                    break 'subsets;
                }
                let mut skip = omitted.iter().map(|&i| blocked[i].0).peekable();
                let roots = component
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| skip.next_if_eq(i).is_none())
                    .flat_map(|(_, g)| g.roots.clone())
                    .collect();
                if let Ok(selected) = self.solve_with_budget(&roots, installed, budget) {
                    for &i in &omitted {
                        let (at, failure) = blocked[i];
                        refuse(plan, &component[at].roots, &failure.reason);
                    }
                    return Ok(Some(Group::validated(roots, selected)));
                }
                // Advance to the next sorted combination of omitted indices.
                let Some(i) = (0..count).rfind(|&i| omitted[i] < blocked.len() - count + i) else {
                    break;
                };
                let next = omitted[i] + 1;
                for (at, index) in omitted[i..].iter_mut().zip(next..) {
                    *at = index;
                }
            }
        }
        if *budget == 0 {
            return Err(
                "dependency subgroup resolution limit reached; try fewer requests or pin a version",
            );
        }
        Ok(None)
    }

    fn solve<'s>(
        &'s self,
        roots: &Roots<'s>,
        installed: &Selected<'s>,
    ) -> Result<Selected<'s>, String> {
        let mut budget = MAX_STATES;
        self.solve_with_budget(roots, installed, &mut budget)
            .map_err(|failure| failure.reason)
    }

    /// Depth-first search over version choices, not over mods on the call stack. New requirements
    /// can narrow an earlier choice; undo it and retry instead of keeping stale outgoing edges.
    fn solve_with_budget<'s>(
        &'s self,
        roots: &Roots<'s>,
        installed: &Selected<'s>,
        budget: &mut usize,
    ) -> Result<Selected<'s>, SolveFailure<'s>> {
        let mut selected = Selected::new();
        let mut stack = Vec::<(&str, std::vec::IntoIter<&Candidate<'_>>)>::new();
        let mut failure = None;
        let mut replacements = BTreeSet::new();
        while *budget > 0 {
            *budget -= 1;
            let step = (|| {
                let mut needs: BTreeMap<_, _> = roots
                    .iter()
                    .map(|(&id, pins)| {
                        (
                            id,
                            Need {
                                pins: pins.as_slice(),
                                ranges: Vec::new(),
                            },
                        )
                    })
                    .collect();
                for (&id, &c) in &selected {
                    for range in &c.required {
                        range.parsed.as_ref().map_err(Clone::clone)?;
                        needs
                            .entry(range.row.id.as_str())
                            .or_insert(Need {
                                pins: &[],
                                ranges: Vec::new(),
                            })
                            .ranges
                            .push((id, range));
                    }
                }
                // Optional ranges constrain a selected target, or a target already installed.
                // They never add a target to the closure. Check the fixed installed owners
                // only once no pending selection can replace them.
                if needs.keys().all(|id| selected.contains_key(id)) {
                    let mut optional_refusal = None;
                    for (&owner, c) in &selected {
                        for range in &c.optional {
                            let target = range.row.id.as_str();
                            let need = needs.get_mut(target);
                            let installed = installed.get(target);
                            if need.is_none() && installed.is_none() {
                                continue;
                            }
                            let req = match &range.parsed {
                                Ok(req) => req,
                                Err(error) => {
                                    optional_refusal.get_or_insert_with(|| error.clone());
                                    continue;
                                }
                            };
                            if let Some(need) = need {
                                need.ranges.push((owner, range));
                            } else if let Some(row) = installed
                                && !row.number.as_ref().is_some_and(|n| req.matches(n))
                            {
                                replacements.insert(target);
                                optional_refusal.get_or_insert_with(|| {
                                    format!(
                                        "{target} {} is installed, but {owner} needs {}",
                                        row.version, range.row.version
                                    )
                                });
                            }
                        }
                    }
                    if let Some(error) = optional_refusal {
                        return Err(error);
                    }
                    for (&owner, &c) in installed {
                        if !selected.contains_key(owner) {
                            for range in c.required.iter().chain(&c.optional) {
                                let target = range.row.id.as_str();
                                if let Some(need) = needs.get_mut(target) {
                                    let req = range.parsed.as_ref().map_err(Clone::clone)?;
                                    if !selected[target]
                                        .number
                                        .as_ref()
                                        .is_some_and(|n| req.matches(n))
                                    {
                                        replacements.insert(owner);
                                    }
                                    need.ranges.push((owner, range));
                                }
                            }
                        }
                    }
                }
                for (&id, &chosen) in &selected {
                    if let Some(need) = needs.get(id)
                        && !need.accepts(chosen)
                    {
                        // Distinguish a true disjoint range from a choice another branch can undo.
                        self.choices(id, need)?;
                        return Err(format!(
                            "{id} {} does not fit all requirements ({})",
                            chosen.version,
                            need.describe()
                        ));
                    }
                }
                let next = roots
                    .keys()
                    .chain(needs.keys())
                    .find(|id| !selected.contains_key(**id));
                if let Some(&id) = next {
                    let mut choices = self.choices(id, &needs[id])?;
                    if let Some(at) = choices.iter().position(|c| c.installed) {
                        choices[..=at].rotate_right(1);
                    }
                    return Ok(Some((id, choices.into_iter())));
                }
                let nodes: Vec<_> = selected.keys().copied().collect();
                let edges: Vec<_> = selected
                    .values()
                    .map(|c| {
                        c.required
                            .iter()
                            .filter_map(|r| nodes.binary_search(&r.row.id.as_str()).ok())
                            .collect()
                    })
                    .collect();
                if let Some(cycle) = required_cycle(&edges) {
                    let names: Vec<_> = cycle
                        .into_iter()
                        .map(|i| format!("{} {}", nodes[i], selected[nodes[i]].version))
                        .collect();
                    return Err(format!("required dependency cycle: {}", names.join(" -> ")));
                }
                Ok(None)
            })();
            match step {
                Ok(None) => return Ok(selected),
                Ok(Some(frame)) => stack.push(frame),
                Err(reason) => {
                    failure.get_or_insert(reason);
                }
            }
            while let Some((id, choices)) = stack.last_mut() {
                if let Some(next) = choices.next() {
                    selected.insert(*id, next);
                    break;
                }
                selected.remove(id);
                stack.pop();
            }
            if stack.is_empty() {
                return Err(SolveFailure {
                    reason: failure
                        .unwrap_or_else(|| "no consistent versions for the requested mods".into()),
                    replacements,
                });
            }
        }
        Err(SolveFailure {
            reason: format!(
                "{}: dependency resolution limit reached; try fewer requests or pin a version",
                roots.keys().copied().collect::<Vec<_>>().join(", ")
            ),
            replacements,
        })
    }
}

/// Check selected versions as exact pins, reusing the install catalog per Minecraft release and side.
/// This has no installed files and assumes native SDK fit: a publisher can't know a player's build.
/// Package side is a join contract; only dependency side filters client and dedicated installations.
pub fn uninstallable_versions(index: &Index, include: impl Fn(&Version) -> bool) -> Vec<String> {
    let mut by_game = BTreeMap::<_, Vec<_>>::new();
    for p in index.projects.iter().filter(|p| p.kind.is_mod()) {
        for v in p.versions.iter().filter(|v| include(v)) {
            by_game
                .entry(v.game.as_str())
                .or_default()
                .push((p.id.as_str(), v.version.as_str()));
        }
    }
    let mut problems = Vec::new();
    for (game, versions) in by_game {
        let present = Present::new();
        let installed = Selected::new();
        for (side, label) in [(Side::Client, "client"), (Side::Server, "dedicated server")] {
            let catalog = Catalog::new(index, game, side, |_, _| true, &present);
            for &(id, version) in &versions {
                let roots = Roots::from([(id, vec![version])]);
                if let Err(reason) = catalog.solve(&roots, &installed) {
                    problems.push(format!("{id} {version} ({label}): {reason}"));
                }
            }
        }
    }
    problems
}

/// Test the same comparators with an extra exact comparator to opt in to the prerelease. If it
/// then matches, SemVer's prerelease opt-in (rather than the numeric bounds) was the only block.
fn excluded_as_prerelease(range: &VersionReq, version: &semver::Version) -> bool {
    if version.pre.is_empty() {
        return false;
    }
    let mut opted_in = range.clone();
    opted_in.comparators.push(Comparator {
        op: Op::Exact,
        major: version.major,
        minor: Some(version.minor),
        patch: Some(version.patch),
        pre: version.pre.clone(),
    });
    opted_in.matches(version)
}
