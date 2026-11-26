# Add a mod from source

The registry lists only signed, reviewed releases. To install anything else, such as a mod a friend sent
you, use **Add from source** on the **Installed** tab of the launcher's **Mods** page. It offers two
sources:

- **Repository:** a GitHub link or a mod folder on this computer. The launcher builds the mod, and it is
  marked **From source**.
- **Local `.smod` file:** a package already saved on this computer. It is marked **Local**.

Neither source is reviewed by Sunrise. After the launcher reads the source, a warning names exactly where
the code comes from: the repository and commit, the release tag, the folder or the file. Native mod code
runs on your computer with your permissions, so the warning's default is Cancel, and you must choose
**Install unreviewed mod** to go on. Read the source first.

Mods marked From source or Local never update automatically and are left out of **Update all**. To
refresh one, add it from source again, or run `sunrise-launcher mods update-github` (below) for a GitHub
install. A mod installed from the registry is not replaced by an unreviewed copy: remove it first.

To try a mod you are writing, you need no launcher: `cargo sunrise-mod run` builds it and starts the game
with it ([Your first native mod](../../start/first-mod.md)).

## From the command line

```sh
sunrise-launcher mods add-github URL --trust-native-code
```

`--trust-native-code` records your consent to run native code. You can omit it after the launcher
has saved your consent. Before downloading the package or source, the launcher presents the
exact resolved source commit link with its warning so you can review that revision.

## Choose a source

| URL | Selection |
| --- | --- |
| `https://github.com/OWNER/REPO` | Latest release, or the default branch if there is no release |
| `https://github.com/OWNER/REPO/releases/tag/TAG` | Named release |
| `https://github.com/OWNER/REPO/tree/REF` | Branch or tag resolved to a commit |
| `https://github.com/OWNER/REPO/commit/SHA` | Exact commit |

The launcher resolves links through GitHub's public API without sign-in. It reports rate-limit
errors; set `SUNRISE_GITHUB_TOKEN` to use an authenticated rate limit.

A release with a `.smod` asset installs that package. The launcher compares its SHA-256 with
GitHub's digest **when GitHub supplies one**. Without a package asset, it downloads the source
archive at the resolved commit from `codeload.github.com`.

Source discovery follows `native-build FOLDER`: each mod is a flat project with
`sunrise-mod.toml`, `[native]`, `Cargo.toml` and `src/`. Each project pins its Sunrise commit
with `[native] sunrise`. If the archive contains several mods, select one with `--mod ID` or
install all with `--all`. Use `--instance NAME` to choose the destination instance.

The launcher validates packages and packs source projects into `.smod` files before installation.
Validation checks flat source and content layouts and size limits without executing the mod.
This installer rejects content-only release packages.

## Build and update

Build the installed mods with `sunrise-launcher native-build`. Installing a native mod enables
Native mods for the destination.

The launcher saves `sunrise-native-mods/<id>.origin.json` beside the package, recording the
repository, release tag when applicable, commit, asset name and installed file's SHA-256.
`sunrise-launcher mods list` displays the GitHub origin with its tag or abbreviated commit.

- `sunrise-launcher mods update-github [ID] --check` checks for updates without installing them.
- `sunrise-launcher mods update-github [ID]` installs updates with the native-code warning and
  saved consent requirement. Release installs follow the latest release; commit installs follow
  the default branch.
- `sunrise-launcher mods remove ID` removes a mod.

A GitHub install does not overwrite a mod installed from the registry or by hand.

## Trust and safety

Native mods run unsandboxed with full access to your computer. Bundle validation checks packaging,
not whether code is safe, and native mixins compile function-replacement code with the same access
as any other native code.

- **Registry releases** are signed with a GitHub artifact attestation tied to the mod's registered
  repository and Sunrise's shared release workflow, then reviewed for malware, dangerous patterns
  and hidden or obfuscated code before they are listed ([Signed, reviewed releases](../../players/installing-mods.md#signed-reviewed-releases)).
  The launcher verifies the file's SHA-256 hash and attestation against the signed registry index
  and remembers the mod's publisher; if a later release comes from a different publisher, it stops
  and asks before installing. A takedown in the signed index removes a release or a whole project.
  Registry review does not sandbox the code you run: native mods are compiled from source on your
  computer and run with your permissions.
- **Package checks:** a native `.smod` holds a flat Rust project (`sunrise-mod.toml`, `Cargo.toml`,
  `Cargo.lock`, `src/`) and can include `assets/`, `data/` and `icon.png`. The launcher validates
  the bundle without running its code, then builds it for your game build. Registry entries
  distinguish native code with the `native-mod` kind; switching a project between `mod` and
  `native-mod` changes the project's kind in the registry and removes its earlier releases
  ([Publish a mod](../publish.md)).
- **Instance data:** the launcher loads native mods for the instance where you install them, using
  that instance's worlds, options and logs. Other instances select their own versions. This
  separation is not a security boundary: native code can access files outside its instance.

The checks in `horizon-mixins` catch mistakes, not malicious code. See its
[security notes](https://github.com/horizon-rust/horizon-mixins/blob/main/docs/security.md).
