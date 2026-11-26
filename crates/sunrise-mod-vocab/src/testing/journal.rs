//! The transaction journal fake: what a host keeps on each transaction it opens.
use crate::item::transaction::TransactionFrame;
use crate::{ServerHost, TransactionContext, TransactionScopeError, TransactionUnavailable};

// no wire: in-process testing harness state, not a mod boundary payload.
/// A test host's transaction journal, as the game's keeps its storages: the state each open
/// transaction began with, put back when it aborts, and every close in order.
///
/// [`FakeHost`](super::FakeHost) keeps one over its players, level, and attachments. A test
/// host of its own keeps one over its state and delegates
/// [`ServerHost::with_transaction_scope`] and [`ServerHost::transaction_matches`] to
/// [`FakeJournal::with_scope`] and [`FakeJournal::matches`].
#[derive(Debug)]
#[non_exhaustive]
pub struct FakeJournal<S> {
    /// Each open transaction's frame identity and starting state, outermost first.
    kept: Vec<(TransactionFrame, S)>,
    closed: Vec<(usize, bool)>,
    /// Live 1-byte allocation whose address uniquely identifies this journal instance.
    tag: Option<Box<u8>>,
    next: u64,
}

impl<S> Default for FakeJournal<S> {
    fn default() -> Self {
        Self {
            kept: Vec::new(),
            closed: Vec::new(),
            tag: None,
            next: 0,
        }
    }
}

impl<S> Clone for FakeJournal<S> {
    fn clone(&self) -> Self {
        // A cloned host does not inherit live journal frames or identity from the original.
        Self {
            kept: Vec::new(),
            closed: self.closed.clone(),
            tag: None,
            next: 0,
        }
    }
}

impl<S> FakeJournal<S> {
    /// Runs `callback` inside one scoped transaction on `host`, committing on `true` and
    /// restoring the snapshot from `snapshot` on `false` or unwind.
    pub fn with_scope<H: ServerHost>(
        host: &mut H,
        journal: fn(&mut H) -> &mut Self,
        snapshot: fn(&H) -> S,
        restore: fn(&mut H, S),
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn ServerHost,
            &'loan TransactionContext,
        ) -> bool,
    ) -> Result<bool, TransactionScopeError> {
        Self::with_scope_with(host, journal, snapshot, restore, |host, tx| {
            callback(host, tx)
        })
    }

    /// Runs `run` inside one scoped transaction while keeping the concrete host type `H`.
    pub(super) fn with_scope_with<H: ServerHost>(
        host: &mut H,
        journal: fn(&mut H) -> &mut Self,
        snapshot: fn(&H) -> S,
        restore: fn(&mut H, S),
        run: impl for<'loan> FnOnce(&'loan mut H, &'loan TransactionContext) -> bool,
    ) -> Result<bool, TransactionScopeError> {
        struct Scope<'a, H, S> {
            host: &'a mut H,
            journal: fn(&mut H) -> &mut FakeJournal<S>,
            restore: fn(&mut H, S),
            frame: TransactionFrame,
            commit: bool,
        }
        impl<H, S> Drop for Scope<'_, H, S> {
            fn drop(&mut self) {
                let journal = (self.journal)(self.host);
                let Some(at) = journal
                    .kept
                    .iter()
                    .rposition(|(frame, _)| *frame == self.frame)
                else {
                    return;
                };
                let mut back = None;
                while journal.kept.len() > at + 1 {
                    let Some((_, was)) = journal.kept.pop() else {
                        break;
                    };
                    journal.closed.push((journal.kept.len() + 1, false));
                    back = Some(was);
                }
                let Some((_, was)) = journal.kept.pop() else {
                    return;
                };
                journal.closed.push((at + 1, self.commit));
                if !self.commit {
                    back = Some(was);
                }
                if let Some(was) = back {
                    (self.restore)(self.host, was);
                }
            }
        }
        let saved = snapshot(host);
        let frame = {
            let j = journal(host);
            let tag = j.tag.get_or_insert_with(|| Box::new(0));
            let journal_id = std::ptr::from_ref(&**tag) as usize;
            j.next = j
                .next
                .checked_add(1)
                .expect("transaction frame sequence exhausted");
            let frame = TransactionFrame::new(journal_id, j.next, j.kept.len());
            j.kept.push((frame, saved));
            frame
        };
        let mut scope = Scope {
            host,
            journal,
            restore,
            frame,
            commit: false,
        };
        let commit = frame.lend(|tx| run(&mut *scope.host, tx));
        scope.commit = commit;
        drop(scope);
        Ok(commit)
    }

    /// Lends the current frame without opening or closing a transaction.
    pub fn with_current_scope<H: ServerHost>(
        host: &mut H,
        journal: fn(&H) -> &Self,
        callback: &mut dyn for<'loan> FnMut(&'loan mut dyn ServerHost, &'loan TransactionContext),
    ) -> Result<(), TransactionUnavailable> {
        let &(frame, _) = journal(host).kept.last().ok_or(TransactionUnavailable)?;
        frame.lend(|tx| callback(host, tx));
        Ok(())
    }

    /// Whether `tx` names the innermost open transaction (`None` matching when none is open).
    pub fn matches(&self, tx: Option<&TransactionContext>) -> bool {
        match (self.kept.last(), tx) {
            (Some((frame, _)), Some(tx)) => frame.matches(tx),
            (None, None) => true,
            _ => false,
        }
    }

    /// Asserts that `tx` names the innermost open transaction on this journal.
    #[track_caller]
    pub fn assert_current(&self, tx: &TransactionContext) {
        if !self.matches(Some(tx)) {
            let depth = self.depth();
            panic!(
                "transaction context at depth {} does not match the host's innermost open transaction (open depth: {depth})",
                tx.depth()
            );
        }
    }

    /// How many transactions are open.
    pub fn depth(&self) -> usize {
        self.kept.len()
    }

    /// Each transaction closed, in order: the depth it was open at (1 for the outermost), and
    /// whether it committed.
    pub fn closed(&self) -> &[(usize, bool)] {
        &self.closed
    }
}
