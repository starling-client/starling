# Built from the repository root: docker build -f crates/sunrise-telemetry-server/Dockerfile .
FROM rust:1-slim AS build
WORKDIR /src
# The root manifest only for the shared crate's workspace settings (edition, lints).
COPY Cargo.toml Cargo.toml
# Every crate the server's Cargo.lock builds from a path (tests/test_service_images.py checks the list).
COPY crates/sunrise-addresses crates/sunrise-addresses
COPY crates/sunrise-bench crates/sunrise-bench
COPY crates/sunrise-core crates/sunrise-core
COPY crates/sunrise-hooks crates/sunrise-hooks
COPY crates/sunrise-perf-format crates/sunrise-perf-format
COPY crates/sunrise-platform crates/sunrise-platform
COPY crates/sunrise-system crates/sunrise-system
COPY crates/sunrise-telemetry crates/sunrise-telemetry
COPY crates/sunrise-telemetry-server crates/sunrise-telemetry-server
RUN cargo build --release --locked --manifest-path crates/sunrise-telemetry-server/Cargo.toml

FROM debian:stable-slim
COPY --from=build /src/crates/sunrise-telemetry-server/target/release/sunrise-telemetry-server /usr/local/bin/
# Railway's proxy sets X-Real-IP to the client's address; it is the header the server read first before
# SUNRISE_TELEMETRY_CLIENT_IP_HEADER existed, so this keeps the same address for the rate limits (README "Hosting").
ENV SUNRISE_TELEMETRY_DB=/data/telemetry.sqlite3 SUNRISE_TELEMETRY_TRUST_PROXY=1 \
    SUNRISE_TELEMETRY_CLIENT_IP_HEADER=x-real-ip
CMD ["sunrise-telemetry-server"]
