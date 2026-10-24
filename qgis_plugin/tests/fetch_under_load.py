"""Fetches a URL many times in a task of the install while the main thread
holds the GIL as it takes the locks of Qt, as a slot of the panel may.
Exits 0 and prints the fetches once all are done; on a hang, faulthandler
prints the threads and exits 1.

Run by test_install, in a QGIS of its own: fetch_under_load.py PLUGIN URL
SIZE COUNT. The server is in the tests, as one in this process would wait
for the GIL too.
"""

from __future__ import annotations

import faulthandler
import os
import sys

sys.path.insert(0, sys.argv[1])
URL = sys.argv[2]
SIZE = int(sys.argv[3])
COUNT = int(sys.argv[4])

from qgis.core import QgsApplication  # noqa: E402

app = QgsApplication([], False)
app.initQgis()

from qgis.PyQt.QtCore import QObject  # noqa: E402

from topology_rules.core import install  # noqa: E402


class Fetches(install.Task):
    def __init__(self) -> None:
        super().__init__('Fetches')
        self.fetched = 0

    def work(self) -> None:
        for _ in range(COUNT):
            assert len(self.get(URL)) == SIZE
            self.fetched += 1

    def succeed(self) -> None:
        pass


def ignore(name: str) -> None:
    pass


task = Fetches()
ended: list[bool] = []
task.taskCompleted.connect(lambda: ended.append(True))
task.taskTerminated.connect(lambda: ended.append(False))
# Objects all over memory, so that their locks are all the locks of Qt
holders = [QObject() for _ in range(1000)]
for holder in holders:
    holder.objectNameChanged.connect(ignore)
faulthandler.dump_traceback_later(120, exit=True)
QgsApplication.taskManager().addTask(task)
while not ended:
    for holder in holders:
        # Takes the lock of the object and keeps the GIL
        holder.receivers(holder.objectNameChanged)
    app.processEvents()
faulthandler.cancel_dump_traceback_later()
print(task.error or task.fetched, flush=True)
# Without the teardown of QGIS, which has nothing to check here
os._exit(0)
