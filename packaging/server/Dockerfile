# sunrise-server in a container (docs/server.md, section 3). Build context: the repository root:
#   docker build -f packaging/server/Dockerfile -t sunrise-server .
#
# The image holds none of Minecraft's files. Prepare them on the host from your own Minecraft 26.3
# (sunrise-prepare --prepare, see packaging/PLAY.txt) and mount the folder it writes, read-only:
#   docker run -d --name sunrise -p 25565:25565 --stop-timeout 180 \
#     -v sunrise-data:/data \
#     -v /path/to/.local/sunrise-client/26.3:/opt/sunrise/.local/sunrise-client/26.3:ro \
#     -e SUNRISE_EULA=true -e SUNRISE_WHITELIST=true -e SUNRISE_PLAYERS=Alex,Steve -e SUNRISE_OP=Alex \
#     sunrise-server
#
# SUNRISE_EULA=true accepts the Minecraft EULA (https://aka.ms/MinecraftEULA) for this server. The first
# start writes /data/server.toml from SUNRISE_* (EULA, WORLD, SEED, PORT, WHITELIST, PLAYERS, OP); later
# starts use the file. Manage it with `docker exec sunrise sunrise-server status|cmd|backup|logs ...`.
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY . .
# The server build of the game (no sound, no controllers: it needs neither ALSA nor udev) as sunrise-engine.
RUN cargo build --release --locked -p sunrise-game --no-default-features --bin sunrise-client \
    && cargo build --release --locked -p sunrise-server --bin sunrise-server \
    && mkdir /out && cp target/release/sunrise-client /out/sunrise-engine && cp target/release/sunrise-server /out/

FROM debian:bookworm-slim
RUN useradd --system --home-dir /data --shell /usr/sbin/nologin sunrise \
    && mkdir -p /data /opt/sunrise/.local/sunrise-client \
    && chown sunrise:sunrise /data
COPY --from=build /out/ /opt/sunrise/
ENV PATH=/opt/sunrise:$PATH \
    SUNRISE_NO_AUDIO=1
WORKDIR /data
VOLUME /data
EXPOSE 25565
USER sunrise
# SIGTERM makes sunrise-server stop the game cleanly (it saves first): give it --stop-timeout 180.
CMD ["sh", "-c", "test -f server.toml || sunrise-server init --yes || exit 1; exec sunrise-server run"]
