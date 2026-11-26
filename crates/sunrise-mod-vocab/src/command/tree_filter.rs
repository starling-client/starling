use std::sync::Arc;

/// The roots of a player's outgoing command tree, after permission checks and
/// before any listener selections are applied. Root names are borrowed in send order.
// no wire: borrows the outgoing server command tree for this send.
// exhaustive: private fields; constructed only at the command tree send boundary.
#[derive(Clone, Copy)]
pub struct CommandTreeFilter<'a> {
    len: usize,
    name_at: &'a dyn Fn(usize) -> &'a str,
    send: &'a Arc<()>,
}

/// A listener's requested exclusions for this send only. Return it from the
/// callback that created it; it does not select commands in any other tree.
// no wire: returned directly by an in-process command tree callback.
// exhaustive: private fields; constructed only by CommandTreeFilter.
pub struct CommandTreeSelection {
    send: Arc<()>,
    roots: Vec<usize>,
}

impl<'a> CommandTreeFilter<'a> {
    /// The permission-filtered root names, in the order the client would receive them.
    pub fn root_names(&self) -> impl ExactSizeIterator<Item = &'a str> {
        (0..self.len).map(self.name_at)
    }

    /// Select the roots to hide this send. Return the selection from the hook
    /// to apply it. Other listeners may hide more roots, but cannot restore any.
    pub fn hide_where(&self, mut hide: impl FnMut(&str) -> bool) -> Option<CommandTreeSelection> {
        let roots: Vec<_> = self
            .root_names()
            .enumerate()
            .filter_map(|(index, name)| hide(name).then_some(index))
            .collect();
        (!roots.is_empty()).then(|| CommandTreeSelection {
            send: Arc::clone(self.send),
            roots,
        })
    }
}

/// Engine adapter; mods receive a view from the hook and do not construct one.
pub fn command_tree_filter<'a>(
    len: usize,
    name_at: &'a dyn Fn(usize) -> &'a str,
    send: &'a Arc<()>,
) -> CommandTreeFilter<'a> {
    CommandTreeFilter { len, name_at, send }
}

/// Only this send's selections index this send's graph.
pub fn selected_command_roots(
    send: &Arc<()>,
    selection: CommandTreeSelection,
) -> Option<Vec<usize>> {
    Arc::ptr_eq(send, &selection.send).then_some(selection.roots)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_selections_apply_only_to_the_send_that_created_them() {
        let send = Arc::new(());
        let another_send = Arc::new(());
        let name_at = |i: usize| ["first", "second"][i];
        let view = command_tree_filter(2, &name_at, &send);
        assert_eq!(
            selected_command_roots(&send, view.hide_where(|name| name == "second").unwrap()),
            Some(vec![1])
        );
        assert_eq!(
            selected_command_roots(&another_send, view.hide_where(|_| true).unwrap()),
            None
        );
    }
}
