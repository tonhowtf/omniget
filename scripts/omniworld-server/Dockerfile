# Build context is the repository root:
#   docker compose -f scripts/omniworld-server/docker-compose.yml build
FROM rust:1-bookworm@sha256:ade39a710ef85f3506cffcfec3e8a24ba090badef1dff2fb798f71d878833fea AS builder
WORKDIR /build
# Only the server crate: the workspace root is the Tauri app, with heavy system deps.
COPY src-tauri/omniworld-server/ ./
# Detach the crate from the workspace so it builds standalone.
RUN printf '\n[workspace]\n' >> Cargo.toml
RUN cargo build --release

FROM debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc
RUN useradd --system --no-create-home --uid 10001 omniworld
COPY --from=builder /build/target/release/omniworld-server /usr/local/bin/omniworld-server
USER omniworld
EXPOSE 7878
ENTRYPOINT ["/usr/local/bin/omniworld-server"]
