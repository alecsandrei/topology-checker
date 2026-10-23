"""The network cache of the tests is a folder of the run, so that what they
download stays out of the cache of the user."""

from __future__ import annotations

import collections.abc as c
import http.server
import threading
import uuid
from pathlib import Path

import pytest
from qgis.core import (
    QgsBlockingNetworkRequest,
    QgsNetworkAccessManager,
    QgsSettings,
)
from qgis.PyQt.QtCore import QUrl
from qgis.PyQt.QtNetwork import QNetworkRequest

from topology_rules.utils import not_none

from .helpers import NETWORK_CACHE, wait_for


class Echo(http.server.BaseHTTPRequestHandler):
    """Answers each path with the path."""

    def do_GET(self) -> None:
        body = self.path.encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/octet-stream')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args: object) -> None:
        """Keeps the requests out of the output of the tests."""


@pytest.fixture
def server() -> c.Iterator[str]:
    """The address of an Echo server."""
    echo = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Echo)
    thread = threading.Thread(target=echo.serve_forever, daemon=True)
    thread.start()
    yield f'http://127.0.0.1:{echo.server_address[1]}'
    echo.shutdown()
    echo.server_close()


def on_the_main_thread(url: str) -> None:
    """With the manager that QGIS set up as it started."""
    manager = not_none(QgsNetworkAccessManager.instance())
    reply = not_none(manager.get(QNetworkRequest(QUrl(url))))
    wait_for(reply.isFinished)
    reply.deleteLater()


def on_a_new_thread(url: str) -> None:
    """With a manager that is set up for the thread, as for a task."""
    codes: list[QgsBlockingNetworkRequest.ErrorCode] = []

    def fetch() -> None:
        codes.append(
            QgsBlockingNetworkRequest().get(QNetworkRequest(QUrl(url)))
        )

    thread = threading.Thread(target=fetch)
    thread.start()
    thread.join()
    assert codes == [QgsBlockingNetworkRequest.ErrorCode.NoError]


def files_with(folder: Path, text: str) -> list[Path]:
    return [
        path
        for path in folder.rglob('*')
        if path.is_file() and text.encode() in path.read_bytes()
    ]


@pytest.mark.parametrize(
    'fetch', [on_the_main_thread, on_a_new_thread], ids=['main', 'new thread']
)
def test_a_download_is_cached_in_the_folder_of_the_run(
    server: str, fetch: c.Callable[[str], None]
):
    settings = QgsSettings()
    run = Path(settings.fileName()).parent.parent
    folder = Path(str(settings.value(NETWORK_CACHE, '')))
    assert folder.is_absolute()
    assert folder.is_relative_to(run)
    url = f'{server}/{uuid.uuid4().hex}'
    fetch(url)
    cache = not_none(not_none(QgsNetworkAccessManager.instance()).cache())
    try:
        assert cache.metaData(QUrl(url)).isValid()
        assert files_with(folder, url)
    finally:
        cache.remove(QUrl(url))
