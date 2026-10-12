#!/usr/bin/env bash
set -euo pipefail

# This wrapper is run only in the benchmark container. Symlink immutable assets,
# but keep each worker's native preferences and startup addon in its own directory.
gis_distribution="${GIS_GVSIG_HOME:-/opt/gvsig}"
gis_session_app="$1/app"
mkdir -p "$gis_session_app"
cp -as "$gis_distribution/." "$gis_session_app/"
gis_addons="$gis_session_app/preferences/gvSIG/plugins/org.gvsig.scripting.app.mainplugin/2.7.0/scripts/addons"
# Unrelated addon autoruns can continue on background threads after readiness.
# Remove only descriptors in this private profile; keep the stock gap rule.
find "$gis_addons" -name autorun.inf \
  ! -path '*/TopologyRuleMustNotHaveGapsPolygon/*' -delete
gis_addon="$gis_addons/GisBenchmark"
mkdir -p "$gis_addon"
cp /workspace/benchmarks/gis/gvsig_bridge.py "$gis_addon/autorun.py"
cat > "$gis_addon/autorun.inf" <<'EOF'
[Script]
enable = true
main = main
Lang = python

[Unit]
type = Script
name = autorun
EOF
exec bash "$gis_session_app/gvSIG.sh" --consolelogger=false
