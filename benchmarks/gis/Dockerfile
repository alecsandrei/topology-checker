# syntax=docker/dockerfile:1.7
FROM rust:1.95.0-slim-bookworm AS rust_toolchain
FROM ghcr.io/astral-sh/uv:0.12.23 AS uv_toolchain

# Official QGIS 4.2 image, pinned to its Linux amd64 manifest.
FROM qgis/qgis:4.2-questing@sha256:c01e223cb4cfae219765cd251b8dbf1aa216a70588ecea96e052998657b075b5 AS environment
USER root
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    bash build-essential ca-certificates fontconfig fonts-dejavu-core \
    gdal-bin git grass libgdal-dev libssl-dev pkg-config postgresql \
    postgresql-postgis python3 python3-gdal python3-venv xvfb \
    && rm -rf /var/lib/apt/lists/*
COPY --from=rust_toolchain /usr/local/cargo /usr/local/cargo
COPY --from=rust_toolchain /usr/local/rustup /usr/local/rustup
COPY --from=uv_toolchain /uv /usr/local/bin/uv
ENV PATH="/usr/local/cargo/bin:${PATH}" RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
ENV RAYON_NUM_THREADS=1 OMP_NUM_THREADS=1 OPENBLAS_NUM_THREADS=1 QT_QPA_PLATFORM=offscreen
WORKDIR /workspace

FROM environment AS benchmark
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/workspace/target \
    cargo build -p topology-checker-io --features gdal-system --release \
      --example qgis_benchmark --locked \
    && cp target/release/examples/qgis_benchmark /opt/gis-rust-benchmark
COPY benchmarks ./benchmarks
COPY python ./python
RUN uv sync --project benchmarks/gis --frozen --group dev --python /usr/bin/python3 \
    && dpkg-query -W -f='${Package}=${Version}\n' | sort > /opt/gis-packages.lock
ENV GIS_RUST_BINARY=/opt/gis-rust-benchmark
ENV GIS_POSTGIS_DSN="dbname=gis_benchmark user=postgres host=/tmp/gis-pg"
ENV MPLCONFIGDIR=/tmp/gis-matplotlib UV_CACHE_DIR=/tmp/gis-uv-cache
ENTRYPOINT ["bash", "/workspace/benchmarks/gis/container-entrypoint.sh"]
