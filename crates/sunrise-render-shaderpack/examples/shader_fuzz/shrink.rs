//! Failing cases made small while the same invariant still fails, and grouped by invariant and
//! first failing step.

use std::collections::BTreeMap;
use std::time::Instant;

use crate::cases::{Case, CaseId};
use crate::check::{Outcome, Request};

pub type Files = BTreeMap<String, Vec<u8>>;

/// Runs a case: a worker process in a run, the checker itself (or a fake) in tests.
pub trait Evaluate {
    fn eval(&mut self, files: &Files, request: &Request) -> Outcome;
}

impl<F: FnMut(&Files, &Request) -> Outcome> Evaluate for F {
    fn eval(&mut self, files: &Files, request: &Request) -> Outcome {
        self(files, request)
    }
}

fn fails(ev: &mut dyn Evaluate, files: &Files, request: &Request, signature: &str) -> bool {
    ev.eval(files, request)
        .failures
        .iter()
        .any(|f| f.signature() == signature)
}

/// The step that first makes `signature` fail: the case's steps applied one by one, `base` when
/// the case fails so without any.
pub fn first_step(
    ev: &mut dyn Evaluate,
    case: &Case,
    request: &Request,
    signature: &str,
) -> String {
    for n in 0..case.steps.len() {
        if fails(ev, &case.materialize_first(n), request, signature) {
            return match n {
                0 => "base".to_owned(),
                n => case.steps[n - 1].kind.name().to_owned(),
            };
        }
    }
    case.steps
        .last()
        .map_or("base".to_owned(), |s| s.kind.name().to_owned())
}

/// How long a shrink may go on.
#[derive(Clone, Copy)]
pub struct Budget {
    pub evals: usize,
    pub until: Instant,
}

impl Budget {
    fn spend(&mut self) -> bool {
        if self.evals == 0 || Instant::now() >= self.until {
            return false;
        }
        self.evals -= 1;
        true
    }
}

#[derive(Debug, PartialEq)]
pub struct Shrunk {
    /// The steps still needed, with the files they were applied to.
    pub steps: Vec<crate::cases::Step>,
    pub files: Files,
}

/// `case`, smaller, still failing with `signature`: steps dropped, then files, then runs of
/// lines (halving the run until single lines), then files again, each kept only while the
/// failure stays.
///
/// A `refusal` failure is about a program that must load (`request.expect`): without its stage
/// files it would be a missing program, which loads nothing and needs no note (spec 2.5), so
/// the failure would "stay" with the cause gone. Those files are never removed or emptied.
pub fn shrink(
    ev: &mut dyn Evaluate,
    case: &Case,
    request: &Request,
    signature: &str,
    mut budget: Budget,
) -> Shrunk {
    let kept = |path: &str| {
        signature.starts_with("refusal|") && request.expect.iter().any(|p| case.owns(p, path))
    };
    let mut steps = case.steps.clone();
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..steps.len() {
            if !budget.spend() {
                break;
            }
            let mut fewer = steps.clone();
            fewer.remove(i);
            let candidate = Case {
                steps: fewer.clone(),
                ..case.clone()
            };
            if fails(ev, &candidate.materialize(), request, signature) {
                steps = fewer;
                changed = true;
                break;
            }
        }
    }
    let mut files = Case {
        steps: steps.clone(),
        ..case.clone()
    }
    .materialize();
    // A case holds its whole pack (`Case::unmutated`): first every file its programs don't read,
    // at once.
    let unread: Vec<String> = files
        .keys()
        .filter(|p| !kept(p) && !case.reach.contains(*p))
        .cloned()
        .collect();
    if !unread.is_empty() && budget.spend() {
        let mut fewer = files.clone();
        for path in &unread {
            fewer.remove(path);
        }
        if fails(ev, &fewer, request, signature) {
            files = fewer;
        }
    }
    // Biggest first: they hold the most to remove.
    let mut paths: Vec<String> = files.keys().cloned().collect();
    paths.sort_by_key(|p| std::cmp::Reverse(files[p].len()));
    for path in paths.iter().filter(|p| !kept(p)) {
        if !budget.spend() {
            break;
        }
        let mut fewer = files.clone();
        fewer.remove(path);
        if fails(ev, &fewer, request, signature) {
            files = fewer;
        }
    }
    let mut paths: Vec<String> = files.keys().cloned().collect();
    paths.sort_by_key(|p| std::cmp::Reverse(files[p].len()));
    for path in paths {
        let mut lines: Vec<Vec<u8>> = files[&path]
            .split(|&b| b == b'\n')
            .map(<[u8]>::to_vec)
            .collect();
        let mut run = lines.len().div_ceil(2).max(1);
        loop {
            let mut i = 0;
            while i < lines.len() && budget.spend() {
                let mut fewer = lines.clone();
                fewer.drain(i..(i + run).min(lines.len()));
                let text = fewer.join(&b'\n');
                let mut candidate = files.clone();
                let emptied = text.iter().all(u8::is_ascii_whitespace) && kept(&path);
                candidate.insert(path.clone(), text);
                if !emptied && fails(ev, &candidate, request, signature) {
                    lines = fewer;
                    files = candidate;
                } else {
                    i += run;
                }
            }
            if run == 1 || budget.evals == 0 || Instant::now() >= budget.until {
                break;
            }
            run = run.div_ceil(2);
        }
    }
    // Files the lines left empty, or no longer needed.
    let paths: Vec<String> = files.keys().filter(|p| !kept(p)).cloned().collect();
    for path in paths {
        if !budget.spend() {
            break;
        }
        let mut fewer = files.clone();
        fewer.remove(&path);
        if fails(ev, &fewer, request, signature) {
            files = fewer;
        }
    }
    Shrunk { steps, files }
}

/// What a run reports of a case: its seed, pack, dimension, terrain, programs, the bytes of its
/// files with every step applied, and its steps. The dimension and terrain are `None` for a case
/// whose pack ended its case process before they were drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub seed: u64,
    pub pack: String,
    pub dim: Option<usize>,
    pub flat: Option<bool>,
    pub programs: Vec<String>,
    pub bytes: usize,
    pub steps: usize,
}

impl Info {
    /// `case`, whose files with every step applied are `files`.
    pub fn of(case: &Case, files: &Files) -> Self {
        Self {
            bytes: files.values().map(Vec::len).sum(),
            ..Self::sampled(case)
        }
    }

    /// `case` as it is sampled, before its steps run: its bytes are the pack's, without a copy
    /// of the pack.
    pub fn sampled(case: &Case) -> Self {
        Self {
            seed: case.seed,
            pack: case.pack.clone(),
            dim: Some(case.dim),
            flat: Some(case.flat),
            programs: case.programs.clone(),
            bytes: case.files.values().map(Vec::len).sum(),
            steps: case.steps.len(),
        }
    }
}

/// A failing case found by a run: which case (to make it again), what it was, and how it failed.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub id: CaseId,
    pub case: Info,
    pub request: Request,
    pub failure: crate::check::Failure,
    pub step: String,
}

impl Found {
    /// The group: invariant, first failing step, and what about it.
    pub fn group(&self) -> String {
        format!(
            "{}|{}|{}",
            self.failure.invariant, self.step, self.failure.detail
        )
    }
}

/// Failures by group, each group's cases smallest first.
pub fn group(found: Vec<Found>) -> BTreeMap<String, Vec<Found>> {
    let mut groups: BTreeMap<String, Vec<Found>> = BTreeMap::new();
    for f in found {
        groups.entry(f.group()).or_default().push(f);
    }
    for members in groups.values_mut() {
        members.sort_by_key(|f| (f.case.bytes, f.id));
    }
    groups
}
