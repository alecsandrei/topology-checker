#!/usr/bin/env bash
set -euo pipefail
mkdir -p /tmp/gis-pg /tmp/gis-pgdata /tmp/gis-logs
chown postgres:postgres /tmp/gis-pg /tmp/gis-pgdata /tmp/gis-logs
gis_pg_bin="$(dirname "$(find /usr/lib/postgresql -name pg_ctl -type f | sort -V | tail -1)")"
runuser -u postgres -- "$gis_pg_bin/initdb" -D /tmp/gis-pgdata -A trust --no-locale > /tmp/gis-logs/initdb.log
cat >> /tmp/gis-pgdata/postgresql.conf <<'EOF'
listen_addresses = ''
unix_socket_directories = '/tmp/gis-pg'
max_parallel_workers_per_gather = 0
autovacuum = off
checkpoint_timeout = '1h'
EOF
runuser -u postgres -- "$gis_pg_bin/pg_ctl" -D /tmp/gis-pgdata -l /tmp/gis-logs/postgres.log -w start
trap 'runuser -u postgres -- "$gis_pg_bin/pg_ctl" -D /tmp/gis-pgdata -m immediate stop > /dev/null 2>&1 || true' EXIT
runuser -u postgres -- "$gis_pg_bin/createdb" -h /tmp/gis-pg gis_benchmark
runuser -u postgres -- "$gis_pg_bin/psql" -h /tmp/gis-pg -d gis_benchmark -c 'CREATE EXTENSION postgis' > /tmp/gis-logs/extension.log
Xvfb :99 -screen 0 1280x1024x24 -nolisten tcp > /tmp/gis-logs/xvfb.log 2>&1 &
export DISPLAY=:99
uv run --project benchmarks/gis --frozen --no-sync "$@"
