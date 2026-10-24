"""The Python package topology-checker, installed from PyPI into the profile
of QGIS, so that the checks run in QGIS rather than in the executable.

There is no pip: the wheel of this platform is fetched with the network
settings of QGIS, checked against the SHA-256 that PyPI gives, and only the
package is unpacked, as it needs no other. Each version has a folder of its
own, as Windows locks a library that is loaded.

What the panel uses is here. The wheel that fits and its unpacking are in
wheels, the folders of the profile in folders; a test patches a name in the
module that defines it.
"""

from __future__ import annotations

import collections
import dataclasses
import hashlib
import importlib
import json
import platform
import shlex
import sys
import threading
import typing as t
from pathlib import Path

from qgis.core import (
    Qgis,
    QgsNetworkAccessManager,
    QgsNetworkReplyContent,
    QgsTask,
)
from qgis.PyQt.QtCore import (
    QCoreApplication,
    QMetaObject,
    Qt,
    QUrl,
    pyqtSignal,
    pyqtSlot,
)
from qgis.PyQt.QtNetwork import QNetworkReply, QNetworkRequest

from ..utils import not_none, tr
from . import folders, wheels
from .folders import (
    activate,
    folder,
    folder_of,
    installed,
    log,
    outdated,
    remove,
    restart_needed,
    wanted,
)
from .wheels import PACKAGE, Failed, Release, unsupported

__all__ = [
    'PYPI',
    'Failed',
    'InstallTask',
    'Release',
    'ReleaseTask',
    'Task',
    'activate',
    'command',
    'folder',
    'folder_of',
    'installed',
    'log',
    'outdated',
    'remove',
    'restart_needed',
    'size_text',
    'terminal',
    'unsupported',
    'wanted',
]

# The index of the releases; the tests point it to a server of their own
PYPI = 'https://pypi.org/pypi'


# Words and commands


def size_text(size: int) -> str:
    """'3.1 MB', in kilobytes below a tenth of a megabyte."""
    if size >= 100_000:
        return f'{size / 1_000_000:.1f} MB'
    return f'{max(1, round(size / 1000))} KB'


def mac_python() -> str:
    """The Python of QGIS on macOS: the script of the app that sets its
    home, or else the Python itself with its home."""
    app = Path(QCoreApplication.applicationDirPath())
    if (app / 'python').is_file():
        return shlex.quote(str(app / 'python'))
    version = wheels.numbers(platform.python_version())[:2]
    path = app / 'python{}.{}'.format(*version)
    if not path.is_file():
        return 'python3'
    if '.app/Contents/' in path.as_posix():
        home = shlex.quote(sys.base_prefix)
        return f'PYTHONHOME={home} {shlex.quote(str(path))}'
    return shlex.quote(str(path))


def command(version: str | None = None) -> str:
    """The pip command that installs the same as the plugin does, for the
    shell that terminal() names."""
    version = version or folders.wanted()
    target = str(folders.folder_of(version))
    system = platform.system()
    quoted = f'"{target}"' if system == 'Windows' else shlex.quote(target)
    pip = f'-m pip install --no-deps --target {quoted} {PACKAGE}=={version}'
    if system == 'Windows':
        return f'python {pip}'
    if system == 'Darwin':
        return f'{mac_python()} {pip}'
    return f'python3 {pip}'


def forget(url: str) -> None:
    """Drops what QGIS keeps of the URL in its cache. A request that fails
    gets the status and the body of the answer kept from before, and a
    wheel would take room there for nothing."""
    cache = not_none(QgsNetworkAccessManager.instance()).cache()
    if cache is not None:
        cache.remove(QUrl(url))


def terminal() -> str:
    """Where command() is run."""
    if platform.system() == 'Windows':
        return tr('OSGeo4W Shell')
    return tr('Terminal')


# The tasks


@dataclasses.dataclass
class Answer:
    """What a download gave: its body, or why it failed."""

    content: bytes = b''
    error: str = ''
    status: object = None


class Task(QgsTask):
    """A step of the install that logs what it does, live and to the logs,
    and says why it failed.

    Its downloads run on the main thread, and the thread of the task waits
    for them; what it says is shown from the main thread too. So that thread
    makes no connection and has no slot called: Qt calls those while it
    holds a lock, and Python there would wait for the GIL, which the main
    thread may keep while it waits for the lock. A cancel aborts the
    download where it runs.
    """

    logged = pyqtSignal(str)
    failed = pyqtSignal(str)

    def __init__(self, description: str):
        super().__init__(description)
        self.error = ''
        # What the thread of the task said, for the main thread to show
        self.said: collections.deque[tuple[str, Qgis.MessageLevel]] = (
            collections.deque()
        )
        # The download that get() waits for
        self.fetching = ''
        self.reply: QNetworkReply | None = None
        self.answer: Answer | None = None
        self.answered = threading.Event()

    def say(
        self, text: str, level: Qgis.MessageLevel = Qgis.MessageLevel.Info
    ) -> None:
        self.said.append((text, level))
        if threading.current_thread() is threading.main_thread():
            self.flush()
        else:
            # Only posts an event, which takes no lock of Qt's signals
            QMetaObject.invokeMethod(
                self, 'flush', Qt.ConnectionType.QueuedConnection
            )

    @pyqtSlot()
    def flush(self) -> None:
        """Logs and shows what the task said."""
        while self.said:
            text, level = self.said.popleft()
            folders.log(text, level)
            self.logged.emit(text)

    def cancel(self) -> None:
        super().cancel()
        # On the main thread, as the download
        if self.reply is not None:
            self.reply.abort()
        self.answered.set()

    def get(self, url: str, missing: str = '') -> bytes:
        """The body of the URL, fetched with the proxy, SSL and
        authentication settings of QGIS. Raises Failed with missing, when
        given, for a URL that is not found."""
        if threading.current_thread() is threading.main_thread():
            # The download runs on the main thread, which would wait here
            raise RuntimeError('Task.get runs on the thread of the task')
        self.answered.clear()
        self.answer = None
        self.fetching = url
        QMetaObject.invokeMethod(
            self, 'fetch', Qt.ConnectionType.QueuedConnection
        )
        # Also ends on a cancel that did not come through cancel() here
        while not self.answered.wait(0.2) and not self.isCanceled():
            pass
        answer = self.answer
        if self.isCanceled() or answer is None:
            raise Failed(tr('Cancelled'))
        if answer.status == 404 and missing:
            raise Failed(missing)
        if answer.error:
            raise Failed(
                tr('Cannot fetch {url}: {error}').format(
                    url=url, error=answer.error
                )
            )
        return answer.content

    @pyqtSlot()
    def fetch(self) -> None:
        """Starts the download that get() waits for."""
        if self.isCanceled():
            self.answered.set()
            return
        forget(self.fetching)
        request = QNetworkRequest(QUrl(self.fetching))
        request.setAttribute(
            QNetworkRequest.Attribute.CacheLoadControlAttribute,
            QNetworkRequest.CacheLoadControl.AlwaysNetwork,
        )
        request.setAttribute(
            QNetworkRequest.Attribute.RedirectPolicyAttribute,
            QNetworkRequest.RedirectPolicy.NoLessSafeRedirectPolicy,
        )
        manager = not_none(QgsNetworkAccessManager.instance())
        self.reply = reply = not_none(manager.get(request))
        reply.downloadProgress.connect(self.downloaded)
        reply.finished.connect(self.replied)
        if reply.isFinished():
            self.replied()

    @pyqtSlot('qint64', 'qint64')
    def downloaded(self, done: int, total: int) -> None:
        if total > 0:
            self.setProgress(100 * done / total)

    @pyqtSlot()
    def replied(self) -> None:
        reply, self.reply = self.reply, None
        if reply is None:
            return
        forget(self.fetching)
        reply.deleteLater()
        state = QgsNetworkReplyContent(reply)
        if state.error() == QNetworkReply.NetworkError.NoError:
            # data() is bytes at run time; some stubs type it as str
            content = t.cast('bytes', reply.readAll().data())
            self.answer = Answer(content)
        else:
            status = state.attribute(
                QNetworkRequest.Attribute.HttpStatusCodeAttribute
            )
            if isinstance(status, int):
                reason = state.attribute(
                    QNetworkRequest.Attribute.HttpReasonPhraseAttribute
                )
                error = f'HTTP {status} {reason or ""}'.strip()
            else:
                error = state.errorString()
            self.answer = Answer(error=error, status=status)
        self.answered.set()

    def work(self) -> None:
        raise NotImplementedError

    def succeed(self) -> None:
        raise NotImplementedError

    def run(self) -> bool:
        # Errors are kept and not raised, as QGIS 4 does not call back a
        # task whose run raised
        try:
            self.work()
        except Exception as error:
            if self.isCanceled():
                self.error = tr('Cancelled')
            elif isinstance(error, Failed):
                self.error = str(error)
            else:
                self.error = f'{type(error).__name__}: {error}'
            self.say(self.error, Qgis.MessageLevel.Critical)
            return False
        return True

    def finished(self, result: bool) -> None:
        self.flush()
        if result:
            self.succeed()
            return
        if not self.error:
            # Cancelled before it ran
            self.error = tr('Cancelled')
            self.say(self.error, Qgis.MessageLevel.Critical)
        self.failed.emit(self.error)


class ReleaseTask(Task):
    """Finds the wheel of a release for this platform on PyPI, without
    downloading it, so that its file, size, source and SHA-256 can be shown
    before anything is installed."""

    found = pyqtSignal(object)

    def __init__(self, version: str | None = None):
        version = version or folders.wanted()
        super().__init__(
            tr('Finding {package} {version}').format(
                package=PACKAGE, version=version
            )
        )
        self.version = version
        self.url = f'{PYPI}/{PACKAGE}/{version}/json'
        self.release: Release | None = None
        self.reason = wheels.unsupported()

    def work(self) -> None:
        self.say(
            tr('Python {python} on {platform}').format(
                python=platform.python_version(), platform=wheels.described()
            )
        )
        if self.reason is not None:
            raise Failed(self.reason)
        self.say(tr('Fetching {url}').format(url=self.url))
        content = self.get(
            self.url,
            tr('PyPI has no {package} {version}').format(
                package=PACKAGE, version=self.version
            ),
        )
        try:
            data = json.loads(content)
        except ValueError as error:
            raise Failed(
                tr('PyPI sent no JSON: {error}').format(error=error)
            ) from error
        release = wheels.pick(data, self.version)
        self.say(
            tr('Found {file} ({size}), SHA-256 {sha256}').format(
                file=release.filename,
                size=size_text(release.size),
                sha256=release.sha256,
            )
        )
        if self.isCanceled():
            raise Failed(tr('Cancelled'))
        self.release = release

    def succeed(self) -> None:
        self.found.emit(self.release)


class InstallTask(Task):
    """Downloads the wheel of a release, checks its SHA-256, unpacks the
    package into its version folder and loads it. A package that is already
    loaded is replaced at the next start."""

    done = pyqtSignal(str)

    def __init__(self, release: Release):
        super().__init__(
            tr('Installing {package} {version}').format(
                package=PACKAGE, version=release.version
            )
        )
        self.release = release
        self.target = folders.folder_of(release.version)

    def work(self) -> None:
        release = self.release
        # Before the download, which would be for nothing
        folders.unloaded(self.target)
        self.say(
            tr('Downloading {file} ({size})').format(
                file=release.filename, size=size_text(release.size)
            )
        )
        self.say(tr('From {url}').format(url=release.url))
        content = self.get(release.url)
        digest = hashlib.sha256(content).hexdigest()
        if digest != release.sha256:
            # The hashes go to the log; the error is short enough to show
            self.say(
                tr('SHA-256 {digest}, PyPI has {expected}').format(
                    digest=digest, expected=release.sha256
                ),
                Qgis.MessageLevel.Warning,
            )
            raise Failed(tr('The download does not match its SHA-256 on PyPI'))
        self.say(tr('SHA-256 {digest} matches PyPI').format(digest=digest))
        if self.isCanceled():
            raise Failed(tr('Cancelled'))
        unpacked, count = wheels.unpack(
            content, release.version, self.target.parent, self.say
        )
        folders.place(unpacked, self.target)
        self.say(
            tr('Unpacked {count} files into {folder}').format(
                count=count, folder=self.target
            )
        )
        self.say(tr('No other packages installed.'))

    def succeed(self) -> None:
        version = self.release.version
        folders.use(self.target)
        loaded = folders.origin()
        if loaded is not None and not folders.inside(loaded, self.target):
            self.say(
                tr(
                    'Restart QGIS to load {package} {version}, as the one '
                    'in {path} is loaded'
                ).format(package=PACKAGE, version=version, path=loaded.parent)
            )
            self.done.emit(version)
            return
        try:
            importlib.import_module(wheels.MODULE)
        except Exception as error:
            for name in [
                name
                for name in sys.modules
                if name == wheels.MODULE or name.startswith(f'{wheels.MODULE}.')
            ]:
                del sys.modules[name]
            folders.unlisted(self.target)
            folders.uninstall(self.target)
            self.error = tr(
                '{package} {version} does not load: {error}'
            ).format(package=PACKAGE, version=version, error=error)
            self.say(self.error, Qgis.MessageLevel.Critical)
            self.failed.emit(self.error)
            return
        self.say(
            tr('Loaded {package} {version}').format(
                package=PACKAGE, version=version
            )
        )
        self.done.emit(version)
