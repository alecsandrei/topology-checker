#!/usr/bin/env bash
set -euo pipefail
# The cc crate can be an inert build dependency of AEGIS. Its pure-rust feature
# exits before probing or invoking a C compiler. Reject active native backends.
if cargo tree -p topology-checker-io -p topology-checker -p topology-checker-web -e normal,build --locked | rg 'gdal(-sys|-src)? v|proj-sys v|libsqlite3-sys v|simsimd v|zstd-sys v|cmake v'; then
  echo 'Default GIS dependencies unexpectedly require native libraries.' >&2
  exit 1
fi
cargo tree -p topology-checker-io -e features --locked | rg 'aegis feature "pure-rust"'
