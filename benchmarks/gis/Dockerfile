# syntax=docker/dockerfile:1.7
FROM rust:1.95.0-slim-bookworm AS rust_toolchain
FROM ghcr.io/astral-sh/uv:0.12.23 AS uv_toolchain
FROM eclipse-temurin:8-jre AS java_runtime

# Official QGIS 4.2 image, pinned to its Linux amd64 manifest.
FROM qgis/qgis:4.2-questing@sha256:c01e223cb4cfae219765cd251b8dbf1aa216a70588ecea96e052998657b075b5
USER root
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    bash build-essential ca-certificates curl fontconfig fonts-dejavu-core \
    gdal-bin git grass libgdal-dev libssl-dev pkg-config postgresql \
    postgresql-postgis python3 python3-gdal python3-venv unzip xvfb xauth \
    && rm -rf /var/lib/apt/lists/*
COPY benchmarks/gis/gvsig-download-chain.pem /opt/gvsig-download-chain.pem
RUN cat /etc/ssl/certs/ca-certificates.crt /opt/gvsig-download-chain.pem > /tmp/gvsig-ca.pem \
    && curl --cacert /tmp/gvsig-ca.pem \
    --ciphers 'ECDHE+AESGCM:ECDHE+AES:RSA+AESGCM:RSA+AES' -fsSL --retry 3 \
    https://downloads.gvsig.org/download/gvsig-desktop-testing/dists/2.7.0/builds/5016/gvSIG-desktop-2.7.0-5016-final-lin-x86_64.zip \
    -o /tmp/gvsig.zip \
    && echo '05ce17089583c1a3968f2a26c95488529dcd3f7931854532838e8279cf6881e0  /tmp/gvsig.zip' | sha256sum -c - \
    && unzip -q /tmp/gvsig.zip -d /opt \
    && mv /opt/gvSIG-desktop-2.7.0-5016-final-lin-x86_64 /opt/gvsig \
    && rm /tmp/gvsig.zip /tmp/gvsig-ca.pem \
    && rm -rf /opt/gvsig/gvSIG/extensiones/jre
COPY --from=java_runtime /opt/java/openjdk /opt/gvsig/gvSIG/extensiones/jre
COPY --from=rust_toolchain /usr/local/cargo /usr/local/cargo
COPY --from=rust_toolchain /usr/local/rustup /usr/local/rustup
COPY --from=uv_toolchain /uv /usr/local/bin/uv
ENV PATH="/usr/local/cargo/bin:${PATH}" RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
ENV RAYON_NUM_THREADS=1 OMP_NUM_THREADS=1 OPENBLAS_NUM_THREADS=1 QT_QPA_PLATFORM=offscreen
WORKDIR /workspace
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/workspace/target \
    cargo build -p topology-checker-io --features gdal-system --release \
      --example qgis_benchmark --locked \
    && cp target/release/examples/qgis_benchmark /opt/gis-rust-benchmark
COPY benchmarks ./benchmarks
COPY tests/conftest.py tests/test_gis_benchmark.py ./tests/
COPY python ./python
RUN uv sync --project benchmarks/gis --frozen --group dev --python /usr/bin/python3 \
    && dpkg-query -W -f='${Package}=${Version}\n' | sort > /opt/gis-packages.lock
ENV GIS_RUST_BINARY=/opt/gis-rust-benchmark GIS_GVSIG_HOME=/opt/gvsig
ENV GIS_POSTGIS_DSN="dbname=gis_benchmark user=postgres host=/tmp/gis-pg"
ENV MPLCONFIGDIR=/tmp/gis-matplotlib UV_CACHE_DIR=/tmp/gis-uv-cache
ENTRYPOINT ["bash", "/workspace/benchmarks/gis/container-entrypoint.sh"]
