//! Call identity selection shared by staging and the launcher's route boundary.

use super::{Cause, Error};
use crate::{crossing::Rule, internals::Internal, points::plain_path};

/// A selected call uses the resolved type owner, without renaming its catalog entry.
pub(crate) fn nominal(entry: &Internal, rule: &Rule) -> Option<String> {
    if entry.kind == "method" {
        let (_, owner) = crate::internals_table::function_scope(rule, entry)?;
        let (_, method) = entry.path.rsplit_once("::")?;
        Some(format!("{}::{method}", owner?))
    } else {
        Some(plain_path(&entry.path))
    }
}

/// A slot names a nominal owner; the catalog retains the lexical impl address.
/// Only resolved owners can bridge those addresses. Written type names are not identity.
pub(crate) fn enclosing<'a>(
    target: &str,
    entries: &'a [Internal],
    rule: &Rule,
) -> Result<&'a Internal, Error> {
    let requested = target.rsplit_once("::").and_then(|(owner, method)| {
        let (scope, ty) = owner.rsplit_once("::")?;
        let written = syn::parse_str(ty).ok()?;
        Some((rule.resolve(scope, &written)?, method))
    });
    let mut matches = entries.iter().filter(|entry| {
        if !entry.cfg.is_empty() || !matches!(entry.kind, "fn" | "method") {
            return false;
        }
        if plain_path(&entry.path) == target {
            return true;
        }
        if entry.kind != "method" {
            return false;
        }
        let Some((owner, method)) = &requested else {
            return false;
        };
        let Some((_, name)) = entry.path.rsplit_once("::") else {
            return false;
        };
        name == *method
            && crate::internals_table::function_scope(rule, entry)
                .is_some_and(|(_, this)| this.as_ref() == Some(owner))
    });
    let found = matches.next().ok_or(Cause::Missing(
        "E_INJECT_MISSING: no eligible function matched the site target",
    ))?;
    if matches.next().is_some() {
        return Err(Cause::Ambiguous("several impls matched the nominal owner").into());
    }
    Ok(found)
}

/// A call's resolved catalog identity, not its source spelling.
pub struct CallIdentity<'a> {
    pub call: &'a str,
    pub nth: usize,
    pub ctx: &'a str,
}

/// Selection never silently retargets a moved or ambiguous call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Selected(usize),
    Moved(usize),
    Lost,
    Ambiguous,
}

/// Find the unique context for one callee, then check its per-callee ordinal.
/// Duplicate contexts are ambiguous even when the requested ordinal still matches.
pub fn select(calls: &[CallIdentity<'_>], call: &str, nth: usize, ctx: &str) -> Selection {
    let mut matches = calls
        .iter()
        .enumerate()
        .filter(|(_, c)| c.call == call && c.ctx == ctx);
    let Some((index, found)) = matches.next() else {
        return Selection::Lost;
    };
    if matches.next().is_some() {
        return Selection::Ambiguous;
    }
    if found.nth == nth {
        Selection::Selected(index)
    } else {
        Selection::Moved(found.nth)
    }
}

/// Typed until the source-staging boundary supplies the declaration's location.
#[derive(Debug)]
pub(crate) enum Refusal {
    Moved(usize),
    Lost,
}
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Moved(nth) => write!(f, "E_INJECT_MOVED: now at nth={nth}; run inject update"),
            Self::Lost => f.write_str("E_INJECT_LOST: call lost or ambiguous; rerun inject add"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nominal_owner_is_not_a_written_name_and_conflicting_imports_refuse() {
        use crate::crossing::tests::internal;
        let mut method = internal(
            "horizon_client::game::feature::Machine::run",
            "method",
            "impl Machine { fn run(&self) -> u32 }",
        );
        method.refs.insert(
            "Machine".into(),
            "horizon_client::game::first::Machine".into(),
        );
        let mut entries = vec![
            internal(
                "horizon_client::game::first::Machine",
                "struct",
                "struct Machine;",
            ),
            internal(
                "horizon_client::game::second::Machine",
                "struct",
                "struct Machine;",
            ),
            method,
        ];
        assert_eq!(
            enclosing(
                "horizon_client::game::first::Machine::run",
                &entries,
                &Rule::new(&entries)
            )
            .unwrap()
            .path,
            "horizon_client::game::feature::Machine::run",
        );
        assert!(
            enclosing(
                "horizon_client::game::second::Machine::run",
                &entries,
                &Rule::new(&entries)
            )
            .is_err()
        );
        let mut conflicting = internal(
            "horizon_client::game::feature::Machine::other",
            "method",
            "impl Machine { fn other(&self) -> u32 }",
        );
        conflicting.refs.insert(
            "Machine".into(),
            "horizon_client::game::second::Machine".into(),
        );
        entries.push(conflicting);
        assert!(
            enclosing(
                "horizon_client::game::first::Machine::run",
                &entries,
                &Rule::new(&entries)
            )
            .is_err()
        );
        entries.pop();
        let mut duplicate = internal(
            "horizon_client::game::another::Machine::run",
            "method",
            "impl Machine { fn run(&self) -> u32 }",
        );
        duplicate.refs.insert(
            "Machine".into(),
            "horizon_client::game::first::Machine".into(),
        );
        entries.push(duplicate);
        let error = enclosing(
            "horizon_client::game::first::Machine::run",
            &entries,
            &Rule::new(&entries),
        )
        .unwrap_err();
        assert!(matches!(error.cause, Cause::Ambiguous(_)), "{error}");
    }

    #[test]
    fn unresolved_qualified_impl_owner_is_a_typed_refusal() {
        use crate::crossing::tests::internal;
        let mut method = internal(
            "horizon_client::game::feature::super::Machine::run",
            "method",
            "impl super::Machine { fn run(&self) -> u32 }",
        );
        method.refs.insert(
            "super::Machine".into(),
            "horizon_client::game::Machine".into(),
        );
        let entries = [
            internal("horizon_client::game::Machine", "struct", "struct Machine;"),
            method,
        ];
        let rule = Rule::new(&entries);
        assert!(nominal(&entries[1], &rule).is_none());
        let error = enclosing("horizon_client::game::Machine::run", &entries, &rule).unwrap_err();
        assert!(matches!(error.cause, Cause::Missing(_)), "{error}");
    }

    #[test]
    fn identity_selection_table() {
        for (rows, expected) in [
            (vec![("callee", 1, "ctx")], Selection::Selected(0)),
            (
                vec![("other", 0, "ctx"), ("callee", 1, "ctx")],
                Selection::Selected(1),
            ),
            (
                vec![("callee", 0, "new"), ("callee", 2, "ctx")],
                Selection::Moved(2),
            ),
            (vec![("callee", 1, "changed")], Selection::Lost),
            (vec![("other", 1, "ctx")], Selection::Lost),
            (vec![], Selection::Lost),
            (
                vec![("callee", 1, "ctx"), ("callee", 2, "ctx")],
                Selection::Ambiguous,
            ),
        ] {
            let calls: Vec<_> = rows
                .iter()
                .map(|(call, nth, ctx)| CallIdentity {
                    call,
                    nth: *nth,
                    ctx,
                })
                .collect();
            assert_eq!(select(&calls, "callee", 1, "ctx"), expected, "{rows:?}");
        }
    }
}
