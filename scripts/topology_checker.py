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

if __name__ == '__main__':

    here = Path(sys.argv[0]).parent
    binary = here / 'bin' / 'topology-checker.exe'
    gdal_location = Path('./gdal/bin').resolve().as_posix()
    gdal_apps = (here / 'gdal' / 'bin'/ 'gdal' / 'apps').resolve().as_posix()
    gdal_data = (here / 'gdal' / 'bin' / 'gdal-data').resolve().as_posix()

    if gdal_location not in os.environ['PATH']:
        os.environ['PATH'] += os.pathsep + gdal_location
    if gdal_apps not in os.environ['PATH']:
        os.environ['PATH'] += os.pathsep + gdal_apps
    if 'GDAL_DATA' not in os.environ:
        os.environ['GDAL_DATA'] = gdal_data

    subprocess.call(
        [binary, *sys.argv[1:]]
    )
