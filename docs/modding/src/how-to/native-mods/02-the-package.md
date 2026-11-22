# The native package

The project and its `.smod` have the same flat layout, with no mod-id directory level inside the archive:

```text
my_mod/
  sunrise-mod.toml        # the only manifest; mod identity and build pins
  icon.png               # optional launcher icon
  assets/my_mod/
  data/my_mod/
  Cargo.toml             # one crate; Cargo metadata is not mod identity
  Cargo.lock             # registry dependency pins, when needed
  src/lib.rs             # impl Hooks
```

A native source package requires both `Cargo.toml` and `[native]`; inconsistent declarations
are refused. A content-only package omits both. The mod's id and version come only from `sunrise-mod.toml`,
not Cargo's package name or version. An id is a plain `[a-z0-9_]{1,64}` name, also used as its resource namespace.
A `rust-toolchain.toml` in the folder is not packaged: the pinned Sunrise commit decides the toolchain that builds
the mod. `cargo sunrise-mod check`, which runs Cargo in the folder, still reads it.
There is no input `native/` or `rust/` directory, `sunrise-native.json` or hand-written `horizon-mixin.toml`.
The launcher writes the tool's manifest and stages the crate itself; those internal paths are not package layouts.

The manifest keeps the [content fields](../content-mods.md#the-manifest), except that
`api` and `capabilities` are written by `cargo sunrise-mod build` into the package, not by you. A native mod adds:

```toml
[native]
game = "26.3"
sunrise = "<exact 40-character lowercase Sunrise commit>"
```

Never write the commit by hand. `cargo sunrise-mod new <path> --template native` fills it in from the SDK of the
build it pins, and `cargo sunrise-mod update` moves it when you pin a newer build ([Install the mod
tool](../../start/install-tool.md), [In a mod project](../../reference/sdk/03-in-a-project.md#the-pin)).
Leave `[native] mixins` out to use the horizon-mixins revision in the pinned SDK. For a mod that uses mixins,
an explicit `mixins = "<exact 40-character lowercase horizon-mixins commit>"` must match that revision; it
does not override it. See [The pin](../../reference/sdk/03-in-a-project.md#the-pin).
Unknown fields are refused. `icon.png` has no manifest key: when present it must be a valid,
nonzero square PNG, at most 512 × 512 pixels and 256 KiB encoded. It is optional for both content
and code mods; the scaffold does not create one.

To change game code itself, add a [`[[native.mixin]]`](../../concepts/native-route/2-mixins.md) row to
`sunrise-mod.toml` and run `cargo sunrise-mod build`.

## What a mixin cannot target

For an unsupported macro in the game body or a `RETURN` target with `?`, use `HEAD` or hook a call to that
function. Select sites outside closures, and use `AFTER_CALL` instead of `WRAP_OPERATION` to observe a call on a
struct literal. Bind a plain `Self` result using the concrete impl type. If a lower-crate helper needs an import
from a group, refer to the imported name explicitly or use its accessible full path. See
[Mixins: what a mixin cannot target](../../concepts/native-route/2-mixins.md#what-a-mixin-cannot-target) for the
reasons, selector spelling and other alternatives.
