# Block entity attachment

Use `block_entity_on_load` to initialize a mod block entity after attachment and
saved-data application, before it ticks. The hook fires for fresh placement,
saved chunk loads and client chunk reception. It corresponds to NeoForge's
`IBlockEntityExtension.onLoad`.

The event supplies `event.subject: BlockEntitySideScope<'_>`: match
`BlockEntitySideScope::Server(scope)` for a mutable `BlockEntityView` and `&mut dyn ServerHost`
sequentially, or `BlockEntitySideScope::Client(scope)` for a read-only `BlockEntityReadView`
and `&mut dyn ClientHost` sequentially. Both views already see the applied saved data. Dispatch
is keyed by the block entity type ID.

## Delivery rules

- The hook fires once per attached identity, on each logical side. Writing a
  payload to an existing identity does not fire it again.
- Unloading and loading a chunk creates new identities and delivers new notices.
  Vanilla block entities do not receive this hook.
- Notices are drained before block entity ticking. Each drain handles only the
  notices pending when it begins; a listener's replacement block queues a notice
  for a later drain.
- If a pre-tick drain replaces the entity about to tick, that tick waits. The
  replacement receives its own notice before ticking.
- Worlds without subscribers collect no notices.
