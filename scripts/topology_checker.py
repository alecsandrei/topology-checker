"""
Wrapper which adds GDAL to the PATH variable and then calls the binary.

Why is this needed? Because topology-checker gets shipped
with a copy of a GDAL version and the user will most likely
not have the path variable pointing to that GDAL folder.
"""

import sys
import os
from pathlib import Path
import subprocess


GDAL_LOCATION = Path('./gdal/bin').resolve().as_posix()
GDAL_APPS = Path('./gdal/bin/gdal/apps').resolve().as_posix()
BINARY = Path('./bin/topology-checker.exe').resolve().as_posix()

if __name__ == '__main__':
    os.environ['PATH'] += os.pathsep + GDAL_LOCATION
    os.environ['PATH'] += os.pathsep + GDAL_APPS
    subprocess.call(
        [BINARY, *sys.argv[1:]]
    )
