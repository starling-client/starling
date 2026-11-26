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

`cargo sunrise-mod build`, `run` and `check` use the same native libraries. For a library you
are working on locally, pass `--with <path>` for its project folder or `.smod`; repeat the flag
for more libraries. A local copy takes precedence over the registry and still has to satisfy the
declared version. Otherwise the tool takes each required library from the Sunrise mod registry's
signed index, including the library's own requirements, and does the same for a release build. A
library is the registry project whose in-game ID is the one you declared. When several projects
provide that ID, the tool names them and you supply the one to use with `--with`. Before it builds
with a release, the tool checks the release package's size and SHA-256 against the index, and its
GitHub attestation against the project's registered repository and release workflow. It also refuses
an index older than the newest one it accepted on the same computer; a fresh machine, such as a CI
runner, accepts any signed index. To include an
optional library, supply it with `--with`; an optional declaration alone never fetches one from the
registry. The tool tries in-range registry releases newest first and takes the first one that fits.
A library pinned to another Sunrise build must carry a uses list that shows it fits the project's
SDK, and the tool doesn't change its pin. Libraries are resolved in the order they are reached: when a
later requirement rules out a release that was already picked, the tool names both requirements.

`check` type-checks the wired code without producing mod binaries, and `build` packages only the
project itself. `run` puts the resolved native libraries and local content packages supplied with
`--with` alongside the project in the development game. Content-only dependencies don't generate
Rust links.

The [registry index and installers](../reference/registry-index.md#planning-installs-and-removals) use these
same manifest declarations. A new submission must resolve with the listed dependencies on its Minecraft release.
Optional dependencies never install automatically, but an installed or selected target must match the declared range.
[`cargo sunrise-mod check <file.smod>`](../reference/sdk/05-commands.md#check-a-released-package) shows both kinds for an attested release.

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

For each supplied native dependency, the build adds a path dependency on the target's root Cargo
package, using the mod ID as the Cargo alias. The dependency exposes items the target makes public
from that crate; there is no separate API package to declare. Optional native dependencies use a
Cargo feature named exactly as their mod ID and enabled only when that library is supplied.
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
