FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential ca-certificates curl file git pkg-config rpm xz-utils zip \
      libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev \
      libpipewire-0.3-dev libssl-dev clang libclang-dev patchelf xdg-utils \
    && rm -rf /var/lib/apt/lists/*

ARG NODE_VERSION=22.20.0
RUN arch=$(dpkg --print-architecture | sed 's/amd64/x64/') \
    && curl -fsSL "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-${arch}.tar.xz" \
       | tar -xJ -C /usr/local --strip-components=1 \
    && corepack enable

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
ARG RUST_VERSION=1.97.0
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "${RUST_VERSION}" --component rustfmt,clippy \
    && chmod -R a+w "$RUSTUP_HOME" "$CARGO_HOME"

ENV NO_STRIP=1 APPIMAGE_EXTRACT_AND_RUN=1 NODE_OPTIONS=--max-old-space-size=6144 CARGO_TARGET_DIR=/target
WORKDIR /work
