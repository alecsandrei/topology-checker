import os
from pathlib import Path
import argparse
from typing import NamedTuple


HERE = __file__
TOPOLOGY_CHECKER = Path(__file__).parent


class GdalEnvironmentVariables(NamedTuple):
    gdal_home: Path
    gdal_version: tuple[str, str, str]
    proj_lib: Path
    pkg_config_path: Path
    pkg_config_sysroot_dir: Path


def parse_arguments():
    parser = argparse.ArgumentParser(description='Build topology-checker.')


def main():
    try:
        variables = GdalEnvironmentVariables(
            Path(os.environ.get('GDAL_HOME')),
            tuple(os.environ.get('GDAL_VERSION')),
            Path(os.environ.get('PROJ_LIB')),
            Path(os.environ.get('PKG_CONFIG_PATH')),
            Path(os.environ.get('PKG_CONFIG_SYSROOT_DIR')),
        )
    except KeyError as e:
        e.add_note(
            'Please set the environment variable the error is mentioning.'
        )
    