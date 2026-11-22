# Capability answer limits

If a handler answers more than requested, `resource_util::move_resources` rolls back the
whole move and returns `Overanswered`. Use `?` to abort the enclosing transaction too when
its other writes depend on the move succeeding.

- `Overanswered.owner` is `Some` with the mod ID or `"the game"` when a capability can
  attribute a slot-less error. A direct handler or a raw indexed answer may leave it `None`.
  Capabilities still warn once, naming their provider, when an indexed answer exceeds the request.
- Handlers must not move more than the amount passed to `insert_at`, `extract_at`, `insert`,
  or `extract`. Journal their writes in the supplied transaction so an abort can undo them.
- Partial acceptance is not an overanswer. The helper attempts to extract exactly the accepted
  amount. If the source cannot supply it exactly, the step rolls back.
- The separate `move_energy` helper keeps its source-first order: partial acceptance rolls
  back its attempt and retries at the smaller amount.
