#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
npm ci --prefix web
npm run build --prefix web
cargo build -p topology-checker-web --release --locked
bundle="builds/topology-checker-web"
mkdir -p "$bundle"
rm -rf "$bundle/web"
mkdir -p "$bundle/web"
cp -R web/dist/. "$bundle/web/"
if [[ -f target/release/topology-checker-web.exe ]]; then
  cp target/release/topology-checker-web.exe "$bundle/"
else
  cp target/release/topology-checker-web "$bundle/"
fi
cp crates/server/examples/DATA-LICENSE.md "$bundle/DATA-LICENSE.md"
cp README.md "$bundle/README.md"
printf 'Bundle ready in %s\n' "$bundle"
