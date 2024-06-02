"""
Helper script to build topology-checker for Windows. It has a CLI.

What this script does:

    -> Takes as input a target directory;
    -> Copies GDAL_HOME and PROJ_LIB to target directory under gdal/;
    -> Runs cargo build --release;
    -> Copies generated binary to target directory under bin/;
    -> Runs pyinstaller on the spec file in the current directory;
    -> Copies generated binary to target directory;

Output target directory structure will look like this:
    target_dir/
    ├─ bin/
    │  ├─ topology-checker.exe
    ├─ gdal/
    ├─ topology-checker.exe

Example on how to run:
    >>> cd scripts
    >>> python3 -m build ../builds
"""

import tempfile
import os
from pathlib import Path
import argparse
from typing import NamedTuple
import shutil
import subprocess


HERE = Path(__file__).parent
TOPOLOGY_CHECKER = Path(__file__).parent


def parse_arguments():
    parser = argparse.ArgumentParser(description='Build topology-checker.')
    parser.add_argument("target_dir",
                        help="Release build target dir",
                        type=Path)
    return parser.parse_args()


class GdalEnvironmentVariables(NamedTuple):
    gdal_home: Path
    proj_lib: Path


def copy_gdal_to(target_dir: Path):
    try:
        variables = GdalEnvironmentVariables(
            Path(os.environ['GDAL_HOME']),
            Path(os.environ['PROJ_LIB']),
        )
    except KeyError as e:
        e.add_note(
            'Please set the environment variable the error is mentioning.'
        )

    assert isinstance(target_dir, Path)
    if not variables.proj_lib.is_relative_to(variables.gdal_home):
        raise NotADirectoryError(
            'PROJ_LIB should be in the same folder as GDAL_HOME.'
        )
    gdal = target_dir / 'gdal'
    build_args = GdalEnvironmentVariables(
        gdal,
        gdal / variables.proj_lib.relative_to(variables.gdal_home),
    )
    # Copy gdal and pkg_config to src dirname.
    if not build_args.gdal_home.exists():
        shutil.copytree(
            variables.gdal_home,
            build_args.gdal_home,
            dirs_exist_ok=True
        )


def main():
    args = parse_arguments()
    # shutil.rmtree(args.target_dir)

    assert isinstance(args.target_dir, Path)

    copy_gdal_to(args.target_dir)

    call = subprocess.call([
        'cargo',
        'build',
        '--release',
    ])
    if call != 0:
        raise Exception('Failed to build.')

    bin = args.target_dir / 'bin'
    if not bin.exists():
        bin.mkdir()

    release = TOPOLOGY_CHECKER.parent / 'target' / 'release' / 'topology-checker.exe'
    if (path := (bin / 'topology-checker.exe')).exists():
        path.unlink()
    shutil.copy(
        release,
        bin
    )

    subprocess.call([
        'pyinstaller',
        HERE / 'topology_checker.spec'
    ])

    wrapper = HERE / 'dist' / 'topology-checker.exe'
    if (path := (args.target_dir / 'topology-checker.exe')).exists():
        path.unlink()
    shutil.copy(
        wrapper,
        args.target_dir
    )

    print('Zipping up the release.')
    with tempfile.TemporaryDirectory() as dir:
        dir = Path(dir)
        name = f'topology_checker_v{parse_version()}'
        out_zip = (HERE / name).with_suffix('.zip')
        if (path := (args.target_dir / out_zip.name)).exists():
            path.unlink()
        shutil.make_archive(
            base_name=name,
            format='zip',
            root_dir=args.target_dir,
            verbose=True,
        )
        shutil.move(
            out_zip,
            args.target_dir
        )


def parse_version():
    with open(HERE / '../Cargo.toml', mode='r', encoding='utf-8') as f:
        for line in f.readlines():
            if line.startswith('version'):
                return ''.join(filter(str.isdigit, line))


if __name__ == '__main__':
    main()
