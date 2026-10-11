# Manual Python release

The Python package's version comes from `crates/python/Cargo.toml`, currently
`0.1.0`. Other Rust applications keep their workspace release versions.
Maturin 1.15.0 supplies dynamic Python version metadata, following the
[Maturin metadata documentation](https://www.maturin.rs/metadata.html).

The Python packages workflow builds regular CPython ABI3 wheels for Linux
x86_64 (manylinux2014, glibc 2.17), Windows x86_64, macOS Intel and macOS Apple
Silicon. These wheels support Python 3.10 through 3.14. Free-threaded CPython,
PyPy, musl Linux and other architectures are outside this release matrix.
An isolated source build requires Rust 1.88 or newer and access to the locked
Rust dependencies, plus the Python build dependencies.

Platform wheel builds use Cargo's locked mode. Maturin trims the workspace in
the source archive, so an isolated source build first removes unused workspace
lock entries. The archive retains the original dependency versions in
`Cargo.lock`; this pruning does not request dependency upgrades.

## Prepare the commit

The release work started with a dirty working tree and an untracked Python
implementation. Review and include these previously untracked paths in the
release commit:

* `crates/python/Cargo.toml` and `crates/python/src/lib.rs`.
* `python/README.md` and all of `python/topology_checker/`, including
  `_native.pyi` and `py.typed`.
* `tests/__init__.py`, `tests/test_api.py` and `tests/test_registration.py`.
* `pyproject.toml`, `uv.lock` and `.github/workflows/python.yml`.

Also include the new release scripts, `tests/conftest.py`, `tests/test_release.py`,
`docs/`, `about.toml`,
`LICENSE` and `THIRD-PARTY-NOTICES.txt`, plus the reviewed changes to `Cargo.lock`,
`README.md` and the binding version. The Python crate relies on ongoing changes
in `crates/core`, including the previously untracked `crates/core/src/violation.rs`
and `crates/core/tests/api.rs`. Review the complete core source and manifest
changes before committing. The root workspace manifest must include
`crates/python`. Preserve and review the other ongoing application changes
separately; `git status` is the authoritative list of outstanding files.

Refresh lockfiles without requesting upgrades after changing the binding
version or Python metadata:

```sh
cargo metadata --no-deps --format-version 1 > /tmp/topology-metadata.json
uv lock
uv sync --locked --group release
```

Generate notices with the exact scanner version:

```sh
cargo install cargo-about --version 0.9.2 --locked --features cli
uv run --locked python scripts/update_notices.py
uv run --locked python scripts/update_notices.py --check
```

`about.toml` scans the locked Python extension graph across all four targets,
excluding dependencies used only for building or development. MIT is preferred
when dependencies allow a choice. The project's source is MIT licensed. The
distribution's SPDX expression additionally covers incorporated dependency
licenses and the LLVM exception. If the script reports a changed expression,
review and update `project.license` before generating the notices. Both full
license files are included in wheel metadata and the source archive according
to the [Python packaging licensing guidance](https://packaging.python.org/en/latest/guides/licensing-examples-and-user-scenarios/).

## Build and download the validated bundle

Commit and push the reviewed release sources before running CI. Use a manual
run on that commit, or push a tag named `python-v0.1.0`. Tag names must exactly
match the binding version. A tag run does not publish anything.

```sh
gh workflow run python.yml --ref master
gh run list --workflow python.yml
gh run watch RUN_ID --exit-status
gh run download RUN_ID --name python-release-bundle --dir release-bundle
```

Replace `RUN_ID` with the successful run for your intended commit. Confirm its
commit SHA in GitHub. Acceptance requires every Python workflow job to pass on
all four platforms and the consolidated artifact to exist. Its contents are
exactly four wheels, one `.tar.gz` source archive and `SHA256SUMS`.

The workflow checks pytest, Ruff, strict Pyright, Rust tests, Clippy, Rust 1.88,
license notice freshness, installed wheels on CPython 3.10 and 3.14, base
operation without GeoPandas, minimum dependencies, and an isolated source build.
The bundle validator verifies archive names and versions, wheel tags,
dependencies, licenses, native binary architectures, stubs and `py.typed`.
Twine's strict check verifies PyPI metadata and README rendering before the
artifact is uploaded. It is a release validation dependency; uploads use uv.

Verify the downloaded bundle from the matching checkout:

```sh
uv run --locked --group release python scripts/validate_release.py release-bundle --tag python-v0.1.0 --verify-checksums
uv run --locked --group release twine check --strict release-bundle/*.whl release-bundle/*.tar.gz
```

You can also run `shasum -a 256 -c SHA256SUMS` inside `release-bundle` on macOS,
or `sha256sum -c SHA256SUMS` on Linux. The Python validator works on all four
platforms. Keep these exact validated archives for uploading, without rebuilding.

## Optional TestPyPI rehearsal and PyPI upload

Run these steps yourself. Keep your credentials in your environment or credential
store, never in this repository or command arguments. `UV_PUBLISH_TOKEN` is
supported by [uv's publishing guide](https://docs.astral.sh/uv/guides/package/).
Use your TestPyPI token for the rehearsal and your PyPI token for the final upload.

```sh
uv publish --publish-url https://test.pypi.org/legacy/ release-bundle/*.whl release-bundle/*.tar.gz
```

For an installation rehearsal, install the wheel's dependencies from PyPI,
then install this exact version from TestPyPI without resolving dependencies:

```sh
uv venv --python 3.10 /tmp/topology-testpypi
uv pip install --python /tmp/topology-testpypi 'shapely>=2,<3' 'pyproj>=3.4,<4' pytest geopandas
uv pip install --python /tmp/topology-testpypi --index-url https://test.pypi.org/simple/ --no-deps --only-binary :all: topology-checker==0.1.0
uv run --no-project --python /tmp/topology-testpypi python scripts/check_install.py
uv run --no-project --python /tmp/topology-testpypi python -m pytest tests
```

After successful verification, select your PyPI credentials and upload the same
five archives:

```sh
uv publish release-bundle/*.whl release-bundle/*.tar.gz
```

Do not pass `SHA256SUMS` to the publisher. PyPI versions are immutable, so a code
or metadata correction after publication needs a new version. No publishing
credentials, package uploads or account changes are part of the automated
workflow.
