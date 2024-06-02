"""
Helper script to build topology-checker for Windows. It has a CLI.

What this script does:

    -> Takes as input a target directory;
    -> Downloads/copies compiled GDAL bin/lib to target directory under gdal/;
    -> Runs cargo build --release;
    -> Copies generated binary to target directory under bin/;
    -> Runs pyinstaller on the spec file in the current directory;
    -> Copies generated binary to target directory;

Output target directory structure will look like this:
    target_dir/
    ├─ bin/
    │  ├─ topology-checker.exe
    ├─ gdal/
    │  ├─ bin
    │  ├─ doc
    │  ├─ include
    │  ├─ lib
    ├─ topology-checker.exe
    ├─ topology-checker_version.zip

Output zip file directory structure will look like this:
    target_dir/
    ├─ bin/
    │  ├─ topology-checker.exe
    ├─ gdal/
    │  ├─ bin
    │  ├─ doc
    ├─ topology-checker.exe

Example on how to run:
1. If you want to download GDAL from gisinternals website.
    >>> cd scripts
    >>> python3 -m build ../builds
2. If you have GDAL_HOME and PKG_CONFIG_PATH environment variables set.
    >>> cd scripts
    >>> python3 -m build --local-gdal ../builds
"""

import zipfile
import tempfile
import os
from pathlib import Path
import argparse
from typing import NamedTuple
import shutil
import subprocess
import requests
from enum import Enum


HERE = Path(__file__).parent
os.chdir(HERE)
TOPOLOGY_CHECKER = Path(__file__).parent
COMPILED_GDAL_BINARIES = 'https://build2.gisinternals.com/sdk/downloads/release-1930-x64-gdal-3-7-3-mapserver-8-0-1.zip'
COMPILED_GDAL_LIBS_HEADERS = 'https://build2.gisinternals.com/sdk/downloads/release-1930-x64-gdal-3-7-3-mapserver-8-0-1-libs.zip'
COMPILED_GDAL_BINARIES_VERSION = '3.7.3'
assert '-'.join(COMPILED_GDAL_BINARIES_VERSION.split('.')) in COMPILED_GDAL_BINARIES


class Bcolors(Enum):
    BLUE = '\033[94m'
    CYAN = '\033[96m'
    GREEN = '\033[92m'
    YELLOW = '\033[93m'
    RED = '\033[91m'


builtin_print = print


class _print:
    def get_color(self, color: str):
        match Bcolors[color.upper()]:
            case Bcolors.BLUE:
                return Bcolors.BLUE.value
            case Bcolors.CYAN:
                return Bcolors.CYAN.value
            case Bcolors.GREEN:
                return Bcolors.GREEN.value
            case Bcolors.YELLOW:
                return Bcolors.YELLOW.value
            case Bcolors.RED:
                return Bcolors.RED.value
            case _:
                return Bcolors.GREEN.value

    def __call__(self, *objects, color='green', **kwargs):
        sep = kwargs.pop('sep', ' ')
        color = self.get_color(color)
        endc = '\033[0m'
        objects = map(str, objects)
        builtin_print(rf'{color}{sep.join(objects)}{endc}', **kwargs)

    def __init__(self):
        # https://stackoverflow.com/questions/12492810/python-how-can-i-make-the-ansi-escape-codes-to-work-also-in-windows
        os.system('color')


print = _print()


def parse_arguments():
    parser = argparse.ArgumentParser(description='Build topology-checker.')
    parser.add_argument('target_dir',
                        help='Release build target directory',
                        type=Path)
    parser.add_argument('--local-gdal',
                        help=('Whether or not to use the local GDAL '
                              + 'installation. GDAL_HOME and PROJ_LIB '
                              + 'environment variables must be set'),
                        action='store_true',
                        default=False)
    return parser.parse_args()


def handle_remote_gdal(gdal_dir: Path):
    if not gdal_dir.exists():
        gdal_dir.mkdir()

    def download_gdal():
        with (
            tempfile.NamedTemporaryFile(delete=True) as binaries,
            tempfile.NamedTemporaryFile(delete=True) as libs_headers
        ):
            print(f'Downloading GDAL bin from {COMPILED_GDAL_BINARIES}', end=' ')
            bin = requests.get(COMPILED_GDAL_BINARIES).content
            binaries.write(bin)
            print('SUCCESS', color='cyan')
            print(f'Downloading GDAL lib from {COMPILED_GDAL_LIBS_HEADERS}', end=' ')
            lib = requests.get(COMPILED_GDAL_LIBS_HEADERS).content
            libs_headers.write(lib)
            print('SUCCESS', color='cyan')
            with (
                zipfile.ZipFile(binaries, 'r') as zip_bin,
                zipfile.ZipFile(libs_headers, 'r') as zip_lib
            ):
                print(f'Extracting GDAL bin archive in {gdal_dir.resolve()}', end=' ')
                zip_bin.extractall(gdal_dir)
                print('SUCCESS', color='cyan')
                print(f'Extracting GDAL lib archive at {gdal_dir.resolve()}', end=' ')
                zip_lib.extractall(gdal_dir)
                print('SUCCESS', color='cyan')

    gdal_pc = gdal_dir / 'gdal.pc'

    def create_gdal_pkg_config_file():
        print(f'Creating gdal.pc file at {gdal_pc}', end=' ')
        lines = [
            'name=gdal',
            'prefix=/usr',
            r'exec_prefix=${prefix}',
            r'libdir=${exec_prefix}/lib',
            r'includedir=${exec_prefix}/include',
            r'datadir=${prefix}/share/${name}',
            r'Name: lib${name}',
            'Description: Geospatial Data Abstraction Library',
            f'Version: {COMPILED_GDAL_BINARIES_VERSION}',
            r'Libs: -L${libdir} -l${name}',
            r'Cflags: -I${includedir}/${name}',
        ]
        with open(gdal_pc, mode='w') as f:
            f.write('\n'.join(lines))
        print('SUCCESS', color='cyan')
        return gdal_pc

    def set_environment_variables():
        print(
            "Setting the GDAL_HOME and PKG_CONFIG_PATH env variables",
            end=' '
        )
        GdalEnvironmentVariables(
            gdal_home=gdal_dir,
            pkg_config_path=gdal_pc.parent
        ).set()
        print('SUCCESS', color='cyan')

    download_gdal()
    create_gdal_pkg_config_file()
    set_environment_variables()


class GdalEnvironmentVariables(NamedTuple):
    gdal_home: Path
    pkg_config_path: Path

    def set(self):
        os.environ['GDAL_HOME'] = self.gdal_home.resolve().as_posix()
        os.environ['PKG_CONFIG_PATH'] = self.pkg_config_path.resolve().as_posix()


def copy_gdal_to(target_dir: Path):
    try:
        variables = GdalEnvironmentVariables(
            Path(os.environ['GDAL_HOME']),
            Path(os.environ['PKG_CONFIG_PATH'])
        )
    except KeyError as e:
        print(f'Did not find env variable {e.args[0]}.', color='red')
        exit(1)
    gdal = target_dir / 'gdal'
    build_args = GdalEnvironmentVariables(
        gdal,
        variables.pkg_config_path
    )
    # Copy gdal to src dirname.
    if not build_args.gdal_home.exists():
        shutil.copytree(
            variables.gdal_home,
            build_args.gdal_home,
            dirs_exist_ok=True
        )


def remove_leftover_files(gdal: Path):
    for file in gdal.rglob('*Zone.Identifier:$DATA'):
        file.unlink()


def build():
    call = subprocess.call([
        'cargo',
        'build',
        '--release',
    ])

    if call != 0:
        raise Exception(
            'Failed to build. cargo build did not return exit status 0.'
        )


def create_binary_wrapper(target_dir: Path):
    subprocess.call([
        'pyinstaller',
        HERE / 'topology_checker.spec'
    ])

    wrapper = HERE / 'dist' / 'topology-checker.exe'
    if (path := (target_dir / 'topology-checker.exe')).exists():
        path.unlink()
    shutil.copy(
        wrapper,
        target_dir
    )


def make_zip(target_dir: Path, out_file: Path):
    exclude_dirs = [
        target_dir / 'gdal' / 'include',
        target_dir / 'gdal' / 'lib',
    ]
    exclude_patterns = [
        '*Zone.Identifier:$DATA',
        '__pycache__/*',
        out_file.name
    ]

    def include_file(file: Path):
        if file.is_dir():
            return False
        if any(file.is_relative_to(dir) for dir in exclude_dirs):
            return False
        elif any(file.match(pat) for pat in exclude_patterns):
            return False
        return True

    print(f'\rCreating zip at {out_file}.')
    files = tuple(filter(include_file, target_dir.rglob('*')))
    len_files = len(files)
    with zipfile.ZipFile(out_file, 'w', zipfile.ZIP_LZMA) as f:
        for i, file in enumerate(files, start=1):
            print(
                f'Archiving {f"{i}/{len_files} files.":<15}',
                end='\r'
            )
            f.write(file, file.relative_to(target_dir))


def main():
    args = parse_arguments()
    assert isinstance(args.target_dir, Path)

    if args.local_gdal is True:
        copy_gdal_to(args.target_dir)
    else:
        gdal = args.target_dir / 'gdal'
        handle_remote_gdal(gdal)

    # Create rust binary with carto
    build()

    # Create wrapper binary with pyinstaller
    create_binary_wrapper(args.target_dir)

    # Create bin folder to store rust binary
    bin = args.target_dir / 'bin'
    if not bin.exists():
        bin.mkdir()

    # Copy rsut binary to bin folder.
    release = TOPOLOGY_CHECKER.parent / 'target' / 'release' / 'topology-checker.exe'
    if (path := (bin / 'topology-checker.exe')).exists():
        path.unlink()
    shutil.copy(
        release,
        bin
    )

    # Create zip file for release
    name = f'topology_checker_v{parse_version()}'
    out_zip = (args.target_dir / name).with_suffix('.zip')
    make_zip(args.target_dir, out_zip)


def parse_version():
    with open(HERE / '../Cargo.toml', mode='r', encoding='utf-8') as f:
        for line in f.readlines():
            if line.startswith('version'):
                return ''.join(filter(str.isdigit, line))


if __name__ == '__main__':
    main()
