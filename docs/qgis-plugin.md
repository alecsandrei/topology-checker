# QGIS plugin

**Topology Rules** adds the rules of topology-checker to QGIS. It runs the checks in the Python package or the [CLI](cli.md), and includes neither. Each [rule](rules.md) is a Processing tool, and a panel runs several rules at once and lists their errors.

## Install

1. In QGIS 3.34 or newer, open **Plugins > Manage and Install Plugins** and install **Topology Rules**.
2. Install the Python package or the CLI:
    - **Install Python package…** in the panel's menu. It first shows the file, its size, URL and SHA-256. Then it downloads the file from PyPI, checks the SHA-256 and unpacks the package into `python/dependencies/topology_rules/<version>` in the QGIS profile. Installs nothing else. Each step goes to the **Topology Rules** log tab and to `install.log`. Needs Python 3.10 or newer, on Windows x64, macOS, or Linux x86_64. A new plugin version offers its package version; QGIS loads it after a restart.
    - The [CLI](install.md#cli), 0.2.0 or newer, such as from `cargo install topology-checker --locked`. The plugin finds it on the `PATH` or in Cargo's bin folder; otherwise choose it in the panel, or set its path under **Settings > Options > Processing > Providers > Topology Rules**.

### Python package by hand

Use the Python of QGIS. `<profile>` is the folder that **Settings > User Profiles > Open Active Profile Folder** opens, and `<version>` is the plugin version. The panel shows the exact command.

Windows, in the OSGeo4W Shell:

```bat
python -m pip install --no-deps --target "<profile>\python\dependencies\topology_rules\<version>" topology-checker==<version>
```

macOS, in Terminal:

```sh
/Applications/QGIS.app/Contents/MacOS/python -m pip install --no-deps --target "<profile>/python/dependencies/topology_rules/<version>" topology-checker==<version>
```

Linux, with pip (for example the `python3-pip` package):

```sh
python3 -m pip install --no-deps --target "<profile>/python/dependencies/topology_rules/<version>" topology-checker==<version>
```

## Panel

Open **Topology Rules** from the Vector menu or the toolbar.

1. Pick a layer and a rule, then **Add rule**. Rules that compare two layers also ask for the other layer. Repeat for more rules.
2. **Run** checks all the ticked rules in one pass.

The gear button next to **?** sets the engine for the panel and the Processing tools: **Automatic** (the Python package where QGIS can use it, else the CLI), **Python package** or **Executable**. Its menu also installs, updates or removes the Python package, and has **Choose executable…**. With only the CLI, the panel offers the package once.

The errors are a tree by rule and kind. Selecting one zooms to it, highlights it and filters the map to its group. **Select features** selects the features an error is about, and **Ignore error** hides an error until you stop ignoring it, also in later runs.

**Style** in the gear menu sets how errors are drawn in every project, Processing outputs included: **Colour by** rule and layer, rule, layer or one colour; **Size** small, normal or large; **Colour ramp** the default or any saved ramp, with **Invert** and **Edit…**. Invalid geometries stay red; no other group gets a colour near it. **Reset style** restores the default.

Under **Options**, **Selected features only** and **Only the visible extent** limit the layer each rule checks, not the layer it is checked against. **Export…** writes the ticked rules as a `topology.toml` for the [CLI](cli.md#check-a-topology-definition), saving layers that are not files of their own in a GeoPackage next to it. **Import…** reads one. The list, the options and the ignored errors are saved with the project.

## Processing

Each rule is a tool named after it, such as **Lines must not have dangles**, in the Topology Rules provider, grouped as Points, Lines, Polygons and All geometries. It adds the errors to the project as point, line and polygon layers. With **Selected features only**, only the selected features are checked. The second layer of a two-layer rule is reprojected to the CRS of the first.

**Check topology definition** runs a `topology.toml`. The tools also run without a window:

```sh
qgis_process run topologyrules:line_must_not_have_dangles --INPUT=roads.gpkg --ERRORS_POINT=errors.gpkg
```

## Tolerance and invalid geometries

The tolerance is in the units of the first layer's CRS. Left empty, it is the CLI default for that CRS: see [Tolerance and CRS](cli.md#tolerance-and-crs).

Features the tool cannot read, such as invalid polygons or coordinates that are not finite, are left out of the run and listed as errors of their own.

## Develop

Lint needs no QGIS: in `qgis_plugin/`, `uv sync`, then `uv run ruff check .` and `uv run ruff format --check .`.

pyright and the tests need a QGIS Python. With Docker, as CI does (`--platform` makes it work on ARM machines too):

```sh
docker run --rm --platform linux/amd64 -v "$PWD":/src -w /src rust:1 cargo build --release -p topology-checker --target-dir target/linux
docker run --rm --platform linux/amd64 -v "$PWD":/src -w /src/qgis_plugin -e TOPOLOGY_CHECKER=/src/target/linux/release/topology-checker -e UV_PROJECT_ENVIRONMENT=/tmp/venv qgis/qgis:stable \
  bash -c 'pip install --break-system-packages uv && uv venv /tmp/venv --system-site-packages --python /usr/bin/python3 && uv sync --locked && uv run pyright && uv run pytest'
```

Tests that need the tool skip without `TOPOLOGY_CHECKER`. With a local QGIS, run `cargo build --release` and point `TOPOLOGY_CHECKER` at `target/release/topology-checker`:

- macOS (checked), with your QGIS app, e.g. `/Applications/QGIS-final-4_2_2.app`: `uv pip install --target /tmp/qgis-pytest pytest pytest-qgis`, then `APP=/Applications/QGIS-final-4_2_2.app/Contents; PYTHONHOME=$APP/Frameworks PYTHONPATH=/tmp/qgis-pytest QT_QPA_PLATFORM=offscreen TOPOLOGY_CHECKER=<absolute path to target/release/topology-checker> $APP/MacOS/python3.12 -m pytest`. pyright does not run on that Python; use Docker.
- Linux and Windows (not checked here): `uv venv --system-site-packages --python <QGIS's python>` (`/usr/bin/python3`, or `python` in the OSGeo4W shell), `uv sync --locked`, then `uv run pytest` and `uv run pyright`.

The modules are listed in `qgis_plugin/topology_rules/__init__.py`.
