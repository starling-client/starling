# {name}

Build and check this native mod with `cargo sunrise-mod build` and `cargo sunrise-mod check`.

## Releases

Follow [Releasing](https://github.com/sunrise-org/sunrise/blob/main/docs/modding/src/how-to/publish.md#releasing)
to register the repository and publish a version. `cargo sunrise-mod release patch` pushes the tag;
the generated `.github/workflows/sunrise-release.yml` calls Sunrise's shared workflow to build and
attest the source package. Commit `Cargo.lock` before releasing to keep the release's dependencies pinned.
