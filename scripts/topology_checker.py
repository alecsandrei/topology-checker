"""Wrapper which adds GDAL to the PATH variable and then calls the binary.

Why is this needed? Because topology-checker gets shipped with a copy of a GDAL version
and the user will most likely not have the path variable pointing to that GDAL folder."""

import sys
import os
from pathlib import Path
import subprocess


def resource_path(relative_path: Path):
    """ Get absolute path to resource, works for dev and for PyInstaller """
    try:
        # PyInstaller creates a temp folder and stores path in _MEIPASS
        base_path = sys._MEIPASS2
    except Exception:
        base_path = Path('.').absolute()
        base_path = os.path.abspath(".")

    return (base_path / relative_path).resolve().as_posix()


GDAL_LOCATION = resource_path(Path('./gdal/bin'))
GDAL_APPS = resource_path(Path('./gdal/bin/gdal/apps'))
BINARY = resource_path(Path('./bin/topology-checker.exe'))

if __name__ == '__main__':
    os.environ['PATH'] += os.pathsep + GDAL_LOCATION
    os.environ['PATH'] += os.pathsep + GDAL_APPS
    subprocess.call(
        [BINARY, *sys.argv[1:]]
    )
