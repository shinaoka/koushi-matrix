FROM node:22.22.3-bookworm

ARG RUST_TOOLCHAIN=1.96.0

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
    libssl-dev \
    libwebkit2gtk-4.1-dev \
    libxdo-dev \
    librsvg2-dev \
    pkg-config \
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

WORKDIR /work

CMD ["bash"]
