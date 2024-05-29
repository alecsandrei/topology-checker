"""
Helper script to build topology-checker for Windows. It has a CLI.

What this script does:

    -> Takes as input a target directory;
    -> Copies GDAL_HOME and PROJ_LIB to target directory under gdal/;
    -> Runs cargo build --release;
    -> Copies generated binary to target directory under bin/;
    -> Runs pyinstaller on the spec file in the current directory;
    -> Copies generated binary to target directory;

Output target directory will look like this:
    target_dir/
    ├─ bin/
    │  ├─ topology-checker.exe
    ├─ gdal/
    ├─ topology-checker.exe
"""

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

    subprocess.call([
        'cargo',
        'build',
        '--release',
    ])

    bin = args.target_dir / 'bin'
    if not bin.exists():
        bin.mkdir()

    shutil.copy(
        TOPOLOGY_CHECKER.parent / 'target' / 'release' / 'topology-checker.exe',
        bin
    )

    subprocess.call([
        'pyinstaller',
        HERE / 'topology_checker.spec'
    ])

    shutil.copy(
        HERE / 'dist' / 'topology-checker.exe',
        args.target_dir
    )


if __name__ == '__main__':
    main()
