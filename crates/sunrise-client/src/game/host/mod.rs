//! The game's side of the host traits: `sunrise-mod-api/src/host/<area>.rs` declares each
//! area's methods and access levels. Build-time generated impls forward to the inherent
//! `Game` functions with the same names. `Host`, `ServerHost` and `ClientHost` are the unions of those traits, so
//! a mod reaches every area through them and the handles.
// The generated impls convert error types that are already equal; TODO(#2273): make the generator skip it.
#![allow(clippy::useless_conversion, clippy::unit_arg)]
use super::Game;
use super::api_values::{TryIntoApi, TryIntoEnginePreservingCount};
use mod_api::__private::game::*;
use mod_api::*;

pub(crate) mod draw;
mod phase_contexts;
pub(crate) mod services;
include!(concat!(env!("OUT_DIR"), "/game_host_generated.rs"));

impl Host for Game<'_> {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(BackendRef::of(self))
    }
}

impl ServerHost for Game<'_> {
    fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
        Some(BackendMutRef::of(self))
    }

    fn with_current_transaction_scope(
        &mut self,
        callback: &mut dyn for<'loan> FnMut(&'loan mut dyn ServerHost, &'loan TransactionContext),
    ) -> Result<(), TransactionUnavailable> {
        let frame = self.current_storage_frame().ok_or(TransactionUnavailable)?;
        frame.lend(|tx| callback(self, tx));
        Ok(())
    }

    fn with_transaction_scope(
        &mut self,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn ServerHost,
            &'loan TransactionContext,
        ) -> bool,
    ) -> Result<bool, TransactionScopeError> {
        self.scoped_storage(|game, tx| callback(game, tx).then_some(()))
            .map(|settled| settled.is_some())
    }

    fn with_scratch_stack_scope(
        &mut self,
        stack: ItemStack,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn ServerHost,
            &'loan TransactionContext,
            &'loan ItemPlace,
        ) -> bool,
    ) -> Result<Option<Option<ItemStack>>, ScratchScopeError> {
        let engine_stack = self
            .stack_boundary_items()
            .ok_or(StackWriteError::Unavailable)
            .and_then(|items| stack.try_into_engine_preserving_count(&items))
            .map_err(ScratchScopeError::Stack)?;
        let taken = self
            .scratch_scope(
                Some(engine_stack.item.clone()),
                engine_stack,
                |game, tx, place| callback(game, tx, place),
            )
            .map_err(ScratchScopeError::Transaction)?;
        Ok(taken.map(|taken| taken.stack.and_then(|stack| stack.try_into_api().ok())))
    }

    fn with_scratch_slots_scope(
        &mut self,
        stack: ItemStack,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn ServerHost,
            &'loan TransactionContext,
            &'loan ItemPlace,
        ) -> bool,
    ) -> Result<Option<(Option<ItemStack>, Option<ItemStack>)>, ScratchScopeError> {
        let engine_stack = self
            .stack_boundary_items()
            .ok_or(StackWriteError::Unavailable)
            .and_then(|items| stack.try_into_engine_preserving_count(&items))
            .map_err(ScratchScopeError::Stack)?;
        let taken = self
            .scratch_scope(None, engine_stack, |game, tx, place| {
                callback(game, tx, place)
            })
            .map_err(ScratchScopeError::Transaction)?;
        Ok(taken.map(|taken| {
            (
                taken.stack.and_then(|stack| stack.try_into_api().ok()),
                taken.overflow.and_then(|stack| stack.try_into_api().ok()),
            )
        }))
    }

    fn transaction_matches(&self, expected: Option<&TransactionContext>) -> bool {
        match (self.current_storage_frame(), expected) {
            (Some(frame), Some(tx)) => frame.matches(tx),
            (None, None) => true,
            _ => false,
        }
    }
}

impl ClientHost for Game<'_> {
    fn client_backend_mut(&mut self) -> Option<ClientBackendMutRef<'_>> {
        Some(ClientBackendMutRef::of(self))
    }
}

impl Backend for Game<'_> {}
impl BackendMut for Game<'_> {}
impl ClientBackendMut for Game<'_> {}
