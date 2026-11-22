# {name}

Build and check this native mod with `cargo sunrise-mod build` and `cargo sunrise-mod check`.

## Stable releases

The generated `.github/workflows/release.yml` plans and builds a `v<version>` tag against your
pinned stable SDK. Its bootstrap action verifies the mod tool before extracting and running the
release scripts built into that tool. The workflow does nothing until `SUNRISE_MOD_RELEASES=true` is set in Actions variables.
Before opting in, set your GitHub repository as `homepage` in `sunrise-mod.toml`, set up the
author signing key and protected signing environment, and follow the
[release setup](https://github.com/sunrise-org/sunrise/blob/main/docs/modding/src/how-to/publish.md#tagged-native-releases-in-github-actions).

`.github/workflows/rebuild.yml` uses the same opt-in. It can add binaries for the current stable release
on a reviewed version's line, from that version's signed `.smod` and lockfile. To request a different
eligible pair, dispatch the workflow on your default branch with the version and stable Sunrise tag.
