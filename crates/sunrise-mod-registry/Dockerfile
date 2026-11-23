# Built from the repository root: docker build -f crates/sunrise-mod-registry/Dockerfile .
FROM rust:1-slim AS build
WORKDIR /src
# The root manifest only for the shared crate's workspace settings (edition, lints).
COPY Cargo.toml Cargo.toml
# Every crate the registry's Cargo.lock builds from a path (tests/test_service_images.py checks the list); cargo needs
# an optional dependency's manifest even with its feature off (sunrise-shader-api).
COPY crates/sunrise-core crates/sunrise-core
COPY crates/sunrise-mod-api-gen crates/sunrise-mod-api-gen
COPY crates/sunrise-game-discovery crates/sunrise-game-discovery
COPY crates/sunrise-mod-vocab crates/sunrise-mod-vocab
# Vocab generation tracks native hook declarations without building gameplay.
COPY crates/sunrise-client/src/game/native_hooks crates/sunrise-client/src/game/native_hooks
COPY crates/sunrise-mod-runtime crates/sunrise-mod-runtime
COPY crates/sunrise-mod-api-macros crates/sunrise-mod-api-macros
COPY crates/sunrise-mod-point-shape crates/sunrise-mod-point-shape
COPY crates/sunrise-mod-source crates/sunrise-mod-source
COPY crates/sunrise-mod-surface crates/sunrise-mod-surface
COPY crates/sunrise-mod-package crates/sunrise-mod-package
COPY crates/sunrise-release-sign crates/sunrise-release-sign
COPY crates/sunrise-shader-api crates/sunrise-shader-api
COPY crates/sunrise-mod-registry crates/sunrise-mod-registry
RUN cargo build --release --locked --manifest-path crates/sunrise-mod-registry/Cargo.toml

FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/crates/sunrise-mod-registry/target/release/sunrise-mod-registry /usr/local/bin/
ENV REGISTRY_DB=/data/registry.sqlite3
# Railway mounts the ratings' volume at /data owned by root: hand it to nobody, then run as nobody (setpriv is
# util-linux's, in every Debian image).
CMD ["sh", "-c", "mkdir -p /data && chown nobody:nogroup /data && exec setpriv --reuid=nobody --regid=nogroup --clear-groups sunrise-mod-registry"]
