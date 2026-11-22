//! Shared Java Identifier dependency order for tag loading.
use std::collections::{HashMap, HashSet};
use sunrise_core::{identifier::Identifier, java_hash_map::JavaHashMap};

/// Admit all required edges first, then optional edges, in the order of the
/// dependency map. A dropped edge is not removed from the actual tag entries.
pub(crate) fn dependency_order<'a, D: Iterator<Item = (&'a str, bool)>>(
    names: impl IntoIterator<Item = &'a str>,
    dependencies: impl Fn(&str) -> D,
) -> Vec<&'a str> {
    // The resource loader builds one map with computeIfAbsent. The sorter
    // inserts its entries, in their iteration order, into a fresh map with put.
    let mut builders = JavaHashMap::default();
    for name in names {
        if let Ok(id) = Identifier::parse(name) {
            builders.compute_if_absent(id, || name);
        }
    }
    let mut contents = JavaHashMap::default();
    for (id, name) in builders {
        contents.put(id, name);
    }
    // The HashMultimap uses a HashSet sized for two values for each parent.
    let mut edges: HashMap<&str, JavaHashMap<Identifier, &str>> = HashMap::new();
    let mut pending = Vec::new();
    let mut seen = HashSet::new();
    for mandatory in [true, false] {
        for (_, &name) in contents.entries() {
            for (id, required) in dependencies(name) {
                if required != mandatory || id == name {
                    continue;
                }
                let Ok(key) = Identifier::parse(id) else {
                    continue;
                };
                if edges
                    .get(name)
                    .is_some_and(|children| children.get(&key).is_some())
                {
                    continue;
                }
                pending.clear();
                seen.clear();
                pending.push(id);
                let mut cyclic = false;
                while let Some(next) = pending.pop() {
                    if next == name {
                        cyclic = true;
                        break;
                    }
                    if seen.insert(next)
                        && let Some(children) = edges.get(next)
                    {
                        pending.extend(children.entries().map(|(_, &child)| child));
                    }
                }
                if !cyclic {
                    edges
                        .entry(name)
                        .or_insert_with(|| JavaHashMap::with_expected_entries(2))
                        .put(key, id);
                }
            }
        }
    }

    let mut visited = HashSet::new();
    let mut sorted = Vec::new();
    let mut stack = Vec::new();
    for (_, &name) in contents.entries() {
        stack.push((name, false));
        while let Some((name, ready)) = stack.pop() {
            if ready {
                sorted.push(name);
            } else if visited.insert(name) {
                stack.push((name, true));
                if let Some(children) = edges.get(name) {
                    let start = stack.len();
                    stack.extend(children.entries().map(|(_, &child)| (child, false)));
                    stack[start..].reverse();
                }
            }
        }
    }
    sorted
}
