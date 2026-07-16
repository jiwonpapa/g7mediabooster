FROM rust:1.96.0-trixie

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        ffmpeg \
        libheif-plugin-aomdec \
        libheif-plugin-aomenc \
        libheif-plugin-libde265 \
        libheif-plugin-x265 \
        libvips-dev \
        libvips-tools \
        perl \
        pkg-config \
        procps \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /work
COPY . .
RUN cargo build --locked --workspace --all-features
RUN cargo test --locked --package g7mb-worker --test load_100 --no-run
RUN cargo build --locked --package xtask

ENTRYPOINT ["bash", "scripts/cgroup-smoke-inner.sh"]
