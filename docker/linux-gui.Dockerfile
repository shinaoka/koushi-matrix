FROM node:22.22.3-bookworm

ARG RUST_TOOLCHAIN=1.96.0
ARG CONDUIT_URL=https://gitlab.com/api/v4/projects/famedly%2Fconduit/jobs/artifacts/master/raw/x86_64-unknown-linux-musl?job=artifacts
ARG TUWUNEL_VERSION=v1.7.1
ARG TUWUNEL_ZST_URL=https://github.com/matrix-construct/tuwunel/releases/download/v1.7.1/v1.7.1-release-all-x86_64-v1-linux-gnu-tuwunel.zst

ENV DEBIAN_FRONTEND=noninteractive \
    RUST_TOOLCHAIN=${RUST_TOOLCHAIN} \
    RUSTUP_HOME=/opt/rustup \
    CARGO_HOME=/opt/cargo \
    PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    dbus-x11 \
    file \
    fontconfig \
    fonts-dejavu-core \
    fonts-noto-color-emoji \
    fonts-noto-core \
    git \
    libayatana-appindicator3-dev \
    libnss-wrapper \
    libssl-dev \
    libwebkit2gtk-4.1-dev \
    libxdo-dev \
    librsvg2-dev \
    pkg-config \
    zstd \
    webkit2gtk-driver \
    xvfb \
  && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- \
    -y \
    --default-toolchain "${RUST_TOOLCHAIN}" \
    --profile minimal \
    --no-modify-path

RUN set -eux; \
    rustup default "${RUST_TOOLCHAIN}"; \
    RUSTC="$(rustup which rustc)"; \
    RUSTDOC="$(rustup which rustdoc)"; \
    RUSTUP_TOOLCHAIN="${RUST_TOOLCHAIN}" RUSTC="$RUSTC" RUSTDOC="$RUSTDOC" cargo install tauri-driver --locked

RUN set -eux; \
    curl --proto '=https' --tlsv1.2 -fsSLo /usr/local/bin/conduit "${CONDUIT_URL}"; \
    chmod 0755 /usr/local/bin/conduit; \
    curl --proto '=https' --tlsv1.2 -fsSLo /tmp/tuwunel.zst "${TUWUNEL_ZST_URL}"; \
    unzstd -f -o /usr/local/bin/tuwunel /tmp/tuwunel.zst; \
    chmod 0755 /usr/local/bin/tuwunel; \
    rm -f /tmp/tuwunel.zst; \
    conduit --version; \
    tuwunel --version

WORKDIR /work

CMD ["bash"]
