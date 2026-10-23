"""The folders of the package in the profile of QGIS: one for each version,
which one loads, deleting the others, and the install log."""

from __future__ import annotations

import configparser
import datetime
import importlib
import os
import shutil
import sys
import threading
from pathlib import Path

from qgis.core import Qgis, QgsApplication, QgsMessageLog

from ..utils import tr
from .wheels import (
    MODULE,
    PACKAGE,
    UNPACKING,
    VERSION,
    Failed,
    numbers,
    problem,
)

# The tab of the Log Messages panel
TAG = 'Topology Rules'
LOG = 'install.log'
METADATA = Path(__file__).parents[1] / 'metadata.txt'

_writing = threading.Lock()


# Versions


def folder() -> Path:
    """Where the package is installed: in the profile, which is writable and
    kept when the plugin is upgraded. Holds a folder for each version and the
    install log."""
    profile = Path(QgsApplication.qgisSettingsDirPath())
    return profile / 'python' / 'dependencies' / 'topology_rules'


def folder_of(version: str) -> Path:
    """The folder that a version is installed into."""
    return folder() / version


def wanted() -> str:
    """The version to install, which is that of the plugin."""
    parser = configparser.ConfigParser(interpolation=None)
    parser.read(METADATA, encoding='utf-8')
    return parser['general']['version'].strip()


def versions() -> list[Path]:
    """The version folders in folder(), newest first."""
    try:
        entries = list(folder().iterdir())
    except OSError:
        return []
    found = [
        path
        for path in entries
        if path.is_dir() and VERSION.fullmatch(path.name)
    ]
    return sorted(found, key=lambda path: numbers(path.name), reverse=True)


def current() -> Path | None:
    """The version folder that the plugin loads: the newest one that loads
    here and is not newer than the plugin."""
    limit = numbers(wanted())
    for path in versions():
        if numbers(path.name) <= limit and problem(path) is None:
            return path
    return None


def installed() -> str | None:
    """The version of the package in folder() that the plugin loads."""
    path = current()
    return None if path is None else path.name


def outdated() -> bool:
    """Whether the installed package is older than the plugin. It is not
    updated on its own: the update is a new install."""
    version = installed()
    return version is not None and numbers(version) < numbers(wanted())


# The folders


def inside(path: str | Path, parent: Path) -> bool:
    """Whether a path is the parent folder or in it, once both are
    absolute."""
    return Path(os.path.abspath(path)).is_relative_to(os.path.abspath(parent))


def stale(path: Path, keep: Path | None) -> str | None:
    """Why a folder in folder() is deleted, or None when it stays. A folder
    for another Python or a newer plugin stays, and is not loaded."""
    if UNPACKING.fullmatch(path.name):
        return tr('left by an install that did not finish')
    if not VERSION.fullmatch(path.name):
        return None
    if not any(path.glob(f'{MODULE}-*.dist-info')):
        return tr('no dist-info')
    if keep is not None and numbers(path.name) < numbers(keep.name):
        return tr('replaced by {version}').format(version=keep.name)
    return None


def delete(path: Path) -> str | None:
    """Deletes a folder, as much of it as can be, and returns why some of it
    is left, or None."""
    try:
        shutil.rmtree(path)
    except FileNotFoundError:
        return None
    except OSError as error:
        shutil.rmtree(path, ignore_errors=True)
        return str(error)
    return None


def uninstall(path: Path) -> str | None:
    """Deletes a version folder, its dist-info first, so that what a locked
    library keeps of it is not taken as installed. Returns why some of it is
    left, or None."""
    for info in path.glob(f'{MODULE}-*.dist-info'):
        delete(info)
    return delete(path)


def origin() -> Path | None:
    """The file of the topology_checker that is imported, if any."""
    module = sys.modules.get(MODULE)
    file = getattr(module, '__file__', None)
    return Path(file) if file else None


def in_use(path: Path) -> bool:
    """Whether the topology_checker that is imported comes from a folder."""
    loaded = origin()
    return loaded is not None and inside(loaded, path)


def unloaded(target: Path) -> None:
    """Raises Failed where the package is imported from a version folder,
    which is then left as it is: Windows locks a library that is loaded, and
    would let only the rest of the folder go."""
    if in_use(target):
        raise Failed(
            tr(
                'Cannot replace {folder} while it is loaded. Restart QGIS '
                'and install again'
            ).format(folder=target)
        )


def place(unpacked: Path, target: Path) -> None:
    """Moves an unpacked folder to its version folder, in place of what is
    there. Raises Failed, and deletes the unpacked folder, where what is there
    is loaded or cannot be deleted."""
    try:
        unloaded(target)
        if target.exists():
            error = uninstall(target)
            if error is not None:
                raise Failed(
                    tr(
                        'Cannot replace {folder}: {error}. Restart QGIS and '
                        'install again'
                    ).format(folder=target, error=error)
                )
        unpacked.rename(target)
    except BaseException:
        delete(unpacked)
        raise


def unlisted(parent: Path) -> None:
    """Takes the folders under parent off sys.path."""
    sys.path[:] = [entry for entry in sys.path if not inside(entry, parent)]


def use(path: Path) -> None:
    """Puts a version folder first on sys.path, so that the next import of
    topology_checker finds it, and drops the other versions from it."""
    unlisted(folder())
    sys.path.insert(0, str(path))
    importlib.invalidate_caches()


def purge(keep: Path | None) -> None:
    """Deletes the folders that are not used any more. The folder of a
    package that is loaded stays, as Windows locks its library."""
    try:
        entries = sorted(folder().iterdir())
    except OSError:
        return
    for path in entries:
        if not path.is_dir() or path == keep or in_use(path):
            continue
        reason = stale(path, keep)
        if reason is None:
            continue
        error = uninstall(path)
        if error is None:
            log(tr('Removed {path}: {reason}').format(path=path, reason=reason))
        else:
            log(
                tr('Could not remove {path}: {error}').format(
                    path=path, error=error
                ),
                Qgis.MessageLevel.Warning,
            )


def activate() -> None:
    """Deletes the versions that are not used, and puts the one that is
    first on sys.path. Runs at plugin load, before anything imports
    topology_checker; it never raises, as the plugin must load."""
    try:
        if not QgsApplication.qgisSettingsDirPath():
            return
        path = current()
        purge(path)
        if path is not None:
            use(path)
    except Exception as error:
        log(
            tr('Could not activate {package}: {error}').format(
                package=PACKAGE, error=error
            ),
            Qgis.MessageLevel.Warning,
        )


def restart_needed() -> bool:
    """Whether QGIS must restart to load what folder() holds: a package
    that is loaded stays loaded, from where it came."""
    if origin() is None:
        return False
    path = current()
    if path is not None:
        return not in_use(path)
    return in_use(folder())


def remove() -> None:
    """Deletes folder() and takes it off sys.path. A package that is loaded
    stays loaded until QGIS restarts, and on Windows so does its library on
    disk; the next start deletes it."""
    home = folder()
    unlisted(home)
    importlib.invalidate_caches()
    if not home.exists():
        return
    for path in versions():
        uninstall(path)
    error = delete(home)
    if error is None:
        log(tr('Removed {folder}').format(folder=home), file=False)
    else:
        log(
            tr('Could not remove all of {folder}: {error}').format(
                folder=home, error=error
            ),
            Qgis.MessageLevel.Warning,
        )


# The log


def log(
    text: str,
    level: Qgis.MessageLevel = Qgis.MessageLevel.Info,
    file: bool = True,
) -> None:
    """Writes a step to the Log Messages panel and to the install log."""
    notify = level != Qgis.MessageLevel.Info
    QgsMessageLog.logMessage(text, TAG, level, notify)
    if not file:
        return
    stamp = datetime.datetime.now().strftime('%Y-%m-%d %H:%M:%S')
    try:
        home = folder()
        home.mkdir(parents=True, exist_ok=True)
        with _writing, (home / LOG).open('a', encoding='utf-8') as handle:
            handle.write(f'{stamp} {text}\n')
    except OSError as error:
        QgsMessageLog.logMessage(
            tr('Cannot write {log}: {error}').format(log=LOG, error=error),
            TAG,
            Qgis.MessageLevel.Warning,
        )
