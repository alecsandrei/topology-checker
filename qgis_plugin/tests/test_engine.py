"""Tests for engine detection, active engine reporting, and performance tip."""

from __future__ import annotations

import types
import typing as t

import pytest
from qgis.core import Qgis

from topology_rules.core import engine
from topology_rules.core.engine import (
    PERFORMANCE_TIP,
    Engine,
    EngineMode,
    active_engine,
    native_available,
)
from topology_rules.panel.dock import Dock
from topology_rules.panel.messages import push_performance_tip


def test_performance_tip_constant():
    assert (
        PERFORMANCE_TIP
        == 'For maximum performance, install topology-checker into your python'
        ' environment, available on pypi'
    )


def test_engine_mode_enums():
    assert EngineMode.CLI == 'CLI'
    assert EngineMode.NATIVE == 'NATIVE'
    assert Engine.CLI is EngineMode.CLI
    assert Engine.NATIVE is EngineMode.NATIVE
    assert str(EngineMode.CLI) == 'CLI'
    assert str(EngineMode.NATIVE) == 'NATIVE'
    assert EngineMode('cli') is EngineMode.CLI
    assert EngineMode('native') is EngineMode.NATIVE


def test_native_available_detection(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(
        'topology_rules.core.engine.native_available', lambda: False
    )
    assert not engine.native_available()
    assert engine.active_engine() == 'CLI'

    monkeypatch.setattr(
        'topology_rules.core.engine.native_available', lambda: True
    )
    assert engine.native_available()
    assert engine.active_engine() == 'NATIVE'


def test_native_available_with_mocked_module(monkeypatch: pytest.MonkeyPatch):
    mock = types.ModuleType('topology_checker')
    mock.Topology = object
    monkeypatch.setitem(__import__('sys').modules, 'topology_checker', mock)
    assert native_available()
    assert active_engine() == 'NATIVE'


def test_dock_surfaces_engine_and_tip(
    bare: Dock, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setattr(
        'topology_rules.core.engine.native_available', lambda: False
    )
    bare.refresh()

    assert bare.active_engine == 'CLI'
    assert bare.active_engine() == 'CLI'
    assert bare.engine == 'CLI'
    assert 'CLI' in bare.engine_label.text()
    assert bare.tip_label.text() == PERFORMANCE_TIP
    assert not bare.tip_label.isHidden()

    monkeypatch.setattr(
        'topology_rules.core.engine.native_available', lambda: True
    )
    bare.refresh()

    assert bare.active_engine == 'NATIVE'
    assert bare.active_engine() == 'NATIVE'
    assert bare.engine == 'NATIVE'
    assert 'NATIVE' in bare.engine_label.text()
    assert bare.tip_label.isHidden()


def test_push_performance_tip(bar: t.Any, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(
        'topology_rules.core.engine.native_available', lambda: False
    )
    push_performance_tip(bar)
    messages = bar.get_messages(Qgis.MessageLevel.Info)
    assert any(PERFORMANCE_TIP in msg for msg in messages)

    calls_before = len(bar.calls)
    monkeypatch.setattr(
        'topology_rules.core.engine.native_available', lambda: True
    )
    push_performance_tip(bar)
    calls_after = len(bar.calls)
    assert calls_after == calls_before
