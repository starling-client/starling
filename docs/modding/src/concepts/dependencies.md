# Dependencies between mods

A mod can publish functions and statics from its root crate for another mod to call.
Declare dependencies in `sunrise-mod.toml`, not `Cargo.toml`:

```toml
[dependencies]
base_mod = "^1"
extra_mod = { version = "^2", optional = true }
```

Requirements use Cargo semver syntax and are checked against each installed mod's `sunrise-mod.toml`
`version`, not its Cargo package version. That version must be semver or the manifest is refused.
Content-only mods use these declarations to validate their requirements and, by default, load
their layers after dependencies. Native packages can depend on content-only packages without
linking anything from them. Missing required dependencies, incompatible versions, and dispatch
cycles refuse loading with the mods named. An optional dependency can be absent, but when installed
it must match its version requirement.

A standalone build wires only the code projects it is given. Other requirements stay in
`sunrise-mod.toml` and are checked against the installed packages at install and startup.
Code that references an unsupplied library still fails to compile.

The [registry index and installers](../reference/registry-index.md#planning-installs-and-removals) use these
same manifest declarations. A new submission must resolve with the listed dependencies on its Minecraft release.
Optional dependencies never install automatically, but an installed or selected target must match the declared range.
`cargo sunrise-mod check <file>` shows both kinds.

A version such as `3.6.30-sunrise.1` is a SemVer prerelease, even when the mod author uses the suffix to name
an ordinary port release. `^3.6` cannot select it. To require that release, declare `"=3.6.30-sunrise.1"`.

Detailed rows also accept `side`, `order`, `reason` and `url`; see the
[complete manifest schema](../reference/manifest.md). Side filters the declaration by installation
before checking it: an integrated server is client, a dedicated server is server. `order = "before"`
or `"none"` changes listener/content dispatch, not compiler staging or actual imports.

The instance's `sunrise-mods/dependency-overrides.toml` can drop a logical requirement or turn it
into a warning, including when a required content package is missing. A removed dependency row no
longer lets a Part attach to that package's effect or block entity type. Overrides cannot remove a
linked native library or compiled package requirement. Selected content and loaded native code share
one logical resolver; warnings and overrides appear on both affected installed Mods rows, alongside
any failed/off status.

Native builds add a path dependency for each supplied library using the mod ID as the Cargo alias
and the target's actual Cargo package name. Optional code libraries use a Cargo feature named
exactly as their mod ID and enabled only when that library is supplied.
Guard calls with `if_installed!`, which builds the block only with that feature and runs it only
when the mod is installed:

```rust,ignore
mod_api::if_installed!(host, extra_mod, {
    extra_mod::register_extra(host);
});
```

Do not define these dependencies or features yourself in `Cargo.toml`.
Public statics must use appropriate synchronization for shared and reentrant hooks; private state in
another mod's hook object is not exposed.

Hook listeners run in descending priority, then resolved dependency dispatch order, with dependency-order
ties kept in bundle order. For source splices, horizon-mixins orders mixins by priority, mod ID, then row ID.
