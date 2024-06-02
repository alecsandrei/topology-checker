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
GDAL_DATA = Path('./gdal/bin/gdal-data').resolve().as_posix()
BINARY = Path('./bin/topology-checker.exe').resolve().as_posix()

if __name__ == '__main__':
    if GDAL_LOCATION not in os.environ['PATH']:
        os.environ['PATH'] += os.pathsep + GDAL_LOCATION
    if GDAL_APPS not in os.environ['PATH']:
        os.environ['PATH'] += os.pathsep + GDAL_APPS
    if 'GDAL_DATA' not in os.environ:
        os.environ['GDAL_DATA'] = GDAL_DATA
    subprocess.call(
        [BINARY, *sys.argv[1:]]
    )
