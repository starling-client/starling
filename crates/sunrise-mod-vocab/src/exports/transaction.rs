pub use item::transaction::{
    HostTransaction, ScratchError, ScratchScopeError, TransactionContext, TransactionError,
    TransactionLifecycle, TransactionScopeError, TransactionUnavailable,
};
mod __private {
    /// A journal frame's process-local identity, built and matched by the game's journal.
    pub use crate::item::transaction::TransactionFrame;
}
