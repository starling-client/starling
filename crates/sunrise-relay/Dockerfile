# The Play with friends relay, for anyone to run (docs/play-with-friends.md, "Running your own relay").
# Build context: the repository root (the workspace's manifests and lockfile):
#   docker build -f crates/sunrise-relay/Dockerfile -t sunrise-relay .
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p sunrise-relay \
    && cp target/release/sunrise-relay /sunrise-relay

FROM debian:bookworm-slim
COPY --from=build /sunrise-relay /usr/local/bin/sunrise-relay
ENV PORT=7000
EXPOSE 7000
USER nobody
CMD ["sunrise-relay"]
