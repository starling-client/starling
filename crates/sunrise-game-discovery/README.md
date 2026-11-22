# Game source helpers

Source-only helpers that the mod API generator and CI's ignored-test inventory share. They read source with `syn`
and never build it. No game feature names, platform dependencies or unsafe code.

- `native_hook_files`: the hand-written files of `game/native_hooks/`, in path order.
- `module_files`: a folder's Rust file stems, sorted and checked to be identifiers.
- `exports`: the vocabulary's export fragments.
- The `sunrise-game-discovery` binary: the ignored-test inventory (`tools/ci/ignored_inventory.py`). It follows each
  crate's `mod` items, `#[path]` included, as the compiler does.

The game's modules are ordinary `mod` items: `game.rs` names each module of `game/<area>/` with a `#[path]`, so an area
folder groups files without becoming a module.
