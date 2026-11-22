//! Dependency ordering for named resource reload listeners.
use crate::Identifier;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
};

/// A problem registering or ordering reload listeners. Match the variants to
/// inspect the involved IDs without parsing the diagnostic message.
// no wire: reports a local reload dependency-graph failure.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReloadOrderError {
    /// A listener name has already been registered.
    DuplicateListener {
        /// The listener's name.
        id: Identifier,
    },
    /// The same listener object has already been registered under another name.
    DuplicateObject {
        /// The listener's name.
        id: Identifier,
        /// The name the object was first registered under.
        registered_as: Identifier,
    },
    /// A dependency named a listener that has not been registered.
    UnknownListener {
        /// The listener's name.
        id: Identifier,
    },
    /// The listed listeners belong to a dependency cycle.
    Cycle {
        /// The names of the listeners in the cycle.
        listeners: Vec<Identifier>,
    },
}
impl std::fmt::Display for ReloadOrderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateListener { id } => write!(f, "duplicate reload listener {id}"),
            Self::DuplicateObject { id, registered_as } => write!(
                f,
                "reload listener {id} is already registered as {registered_as}"
            ),
            Self::UnknownListener { id } => write!(f, "unknown reload listener {id}"),
            Self::Cycle { listeners } => {
                f.write_str("reload listener dependency cycle:")?;
                for id in listeners {
                    write!(f, " {id}")?;
                }
                Ok(())
            }
        }
    }
}
impl std::error::Error for ReloadOrderError {}

pub(crate) struct ReloadOrder<T> {
    listeners: Vec<(Identifier, T)>,
    indices: BTreeMap<Identifier, usize>,
    edges: Vec<(usize, usize)>,
}
impl<T> Default for ReloadOrder<T> {
    fn default() -> Self {
        Self {
            listeners: Vec::new(),
            indices: BTreeMap::new(),
            edges: Vec::new(),
        }
    }
}
impl<T> ReloadOrder<T> {
    pub(crate) fn add_listener(
        &mut self,
        id: Identifier,
        value: T,
    ) -> Result<(), ReloadOrderError> {
        if self.indices.contains_key(&id) {
            return Err(ReloadOrderError::DuplicateListener { id });
        }
        self.indices.insert(id.clone(), self.listeners.len());
        self.listeners.push((id, value));
        Ok(())
    }
    pub(crate) fn add_dependency(
        &mut self,
        before: Identifier,
        after: Identifier,
    ) -> Result<(), ReloadOrderError> {
        let index = |id: Identifier| {
            self.indices
                .get(&id)
                .copied()
                .ok_or(ReloadOrderError::UnknownListener { id })
        };
        let edge = (index(before)?, index(after)?);
        if !self.edges.contains(&edge) {
            self.edges.push(edge);
        }
        Ok(())
    }
    pub(super) fn checkpoint(&self) -> (usize, usize) {
        (self.listeners.len(), self.edges.len())
    }

    pub(super) fn rollback(
        &mut self,
        (listeners, edges): (usize, usize),
        mut discard: impl FnMut(T),
    ) {
        self.edges.truncate(edges);
        let discarded = self.listeners.split_off(listeners);
        for (id, _) in &discarded {
            self.indices.remove(id);
        }
        for (_, value) in discarded {
            discard(value);
        }
    }

    pub(crate) fn registry(&self) -> impl Iterator<Item = (&Identifier, &T)> {
        self.listeners.iter().map(|(id, value)| (id, value))
    }
    pub(crate) fn graph(&self) -> impl Iterator<Item = (&Identifier, &Identifier)> {
        self.edges
            .iter()
            .map(|&(a, b)| (&self.listeners[a].0, &self.listeners[b].0))
    }
    /// Sort listeners in topological order matching NeoForge's `ReloadListenerSort`:
    /// only dangling non-`minecraft` listeners (those with neither a forward path to
    /// nor a backward path from any `minecraft:` listener) are linked after `vanilla`,
    /// and ties follow registration order.
    pub(crate) fn sort(self, vanilla: Option<&Identifier>) -> Result<Vec<T>, ReloadOrderError> {
        let mut outgoing = vec![Vec::new(); self.listeners.len()];
        let mut incoming = vec![Vec::new(); self.listeners.len()];
        for &(a, b) in &self.edges {
            outgoing[a].push(b);
            incoming[b].push(a);
        }
        if let Some(anchor) = vanilla.and_then(|id| self.indices.get(id).copied()) {
            let is_vanilla = |i: usize| self.listeners[i].0.namespace() == "minecraft";
            let reaches_vanilla = |start: usize, adj: &[Vec<usize>]| {
                let mut visited = vec![false; adj.len()];
                let mut stack = vec![start];
                visited[start] = true;
                while let Some(node) = stack.pop() {
                    if is_vanilla(node) {
                        return true;
                    }
                    for &next in &adj[node] {
                        if !visited[next] {
                            visited[next] = true;
                            stack.push(next);
                        }
                    }
                }
                false
            };
            for i in 0..self.listeners.len() {
                if !is_vanilla(i)
                    && !reaches_vanilla(i, &outgoing)
                    && !reaches_vanilla(i, &incoming)
                    && !outgoing[anchor].contains(&i)
                {
                    outgoing[anchor].push(i);
                    incoming[i].push(anchor);
                }
            }
        }
        let mut indegree: Vec<_> = incoming.iter().map(Vec::len).collect();
        let mut ready = BinaryHeap::new();
        for (i, degree) in indegree.iter().enumerate() {
            if *degree == 0 {
                ready.push(Reverse(i));
            }
        }
        let mut order = Vec::with_capacity(self.listeners.len());
        while let Some(Reverse(node)) = ready.pop() {
            order.push(node);
            for &next in &outgoing[node] {
                indegree[next] -= 1;
                if indegree[next] == 0 {
                    ready.push(Reverse(next));
                }
            }
        }
        if order.len() != self.listeners.len() {
            // Follow remaining incoming edges to report the cycle itself,
            // excluding listeners merely downstream of it.
            let mut seen = vec![None; self.listeners.len()];
            let mut path: Vec<usize> = Vec::new();
            let mut node = indegree.iter().position(|&n| n != 0).unwrap();
            loop {
                if let Some(start) = seen[node] {
                    return Err(ReloadOrderError::Cycle {
                        listeners: path[start..]
                            .iter()
                            .map(|&i| self.listeners[i].0.clone())
                            .collect(),
                    });
                }
                seen[node] = Some(path.len());
                path.push(node);
                node = *incoming[node].iter().find(|&&i| indegree[i] != 0).unwrap();
            }
        }
        let mut listeners: Vec<_> = self.listeners.into_iter().map(Some).collect();
        Ok(order
            .into_iter()
            .map(|i| listeners[i].take().unwrap().1)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(value: &str) -> Identifier {
        Identifier::parse(value).unwrap()
    }
    fn graph() -> ReloadOrder<&'static str> {
        let mut graph = ReloadOrder::default();
        for name in [
            "demo:independent",
            "minecraft:vanilla",
            "demo:before",
            "demo:after",
        ] {
            graph.add_listener(id(name), name).unwrap();
        }
        graph
    }
    #[test]
    fn explicit_before_vanilla_does_not_cycle_with_default_order() {
        let mut graph = graph();
        graph
            .add_dependency(id("demo:before"), id("minecraft:vanilla"))
            .unwrap();
        graph
            .add_dependency(id("minecraft:vanilla"), id("demo:after"))
            .unwrap();
        assert_eq!(
            graph.sort(Some(&id("minecraft:vanilla"))).unwrap(),
            [
                "demo:before",
                "minecraft:vanilla",
                "demo:independent",
                "demo:after"
            ]
        );
    }
    #[test]
    fn only_dangling_non_minecraft_listeners_are_linked_after_anchor() {
        let mut graph = ReloadOrder::default();
        for name in [
            "minecraft:anchor",
            "minecraft:extra",
            "demo:after_extra",
            "demo:before",
            "demo:dangling",
        ] {
            graph.add_listener(id(name), name).unwrap();
        }
        graph
            .add_dependency(id("minecraft:extra"), id("demo:after_extra"))
            .unwrap();
        graph
            .add_dependency(id("demo:before"), id("minecraft:anchor"))
            .unwrap();
        assert_eq!(
            graph.sort(Some(&id("minecraft:anchor"))).unwrap(),
            [
                "minecraft:extra",
                "demo:after_extra",
                "demo:before",
                "minecraft:anchor",
                "demo:dangling",
            ]
        );
    }
    #[test]
    fn unknown_dependencies_fail_immediately() {
        assert_eq!(
            graph().add_dependency(id("demo:missing"), id("demo:before")),
            Err(ReloadOrderError::UnknownListener {
                id: id("demo:missing")
            })
        );
    }
    #[test]
    fn cycle_does_not_name_downstream_listeners() {
        let mut graph = graph();
        graph
            .add_dependency(id("demo:before"), id("demo:after"))
            .unwrap();
        graph
            .add_dependency(id("demo:after"), id("demo:before"))
            .unwrap();
        graph
            .add_dependency(id("demo:after"), id("demo:independent"))
            .unwrap();
        let ReloadOrderError::Cycle { listeners } = graph.sort(None).unwrap_err() else {
            panic!("expected cycle")
        };
        assert_eq!(listeners.len(), 2);
        assert!(listeners.contains(&id("demo:before")) && listeners.contains(&id("demo:after")));
    }
}
