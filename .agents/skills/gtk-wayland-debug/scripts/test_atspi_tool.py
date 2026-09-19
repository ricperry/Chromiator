#!/usr/bin/env python3
"""Focused unit tests for the private AT-SPI helper's compact state readback."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import unittest
from unittest.mock import Mock, patch


MODULE_PATH = Path(__file__).with_name("atspi_tool.py")
SPEC = importlib.util.spec_from_file_location("atspi_tool", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
atspi_tool = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = atspi_tool
SPEC.loader.exec_module(atspi_tool)


class FakeState:
    """Expose a small AT-SPI-compatible state set for helper unit tests."""

    def __init__(self, states) -> None:
        """Store the native states that one fake accessible node exposes."""
        self.states = set(states)

    def contains(self, state) -> bool:
        """Report whether the fake node contains one queried AT-SPI state."""
        return state in self.states


class FakeNode:
    """Expose a fake state-bearing accessible object without a desktop session."""

    def __init__(self, states) -> None:
        """Retain the supplied AT-SPI states for `getState`."""
        self.state = FakeState(states)

    def getState(self):
        """Return the fake native state set used by the helper under test."""
        return self.state


class FakePopupNode:
    """Model a named popup descendant and its real accessible parent chain."""

    def __init__(self, name: str, role: str, parent=None) -> None:
        """Retain the minimal AT-SPI identity used by selector and focus tests."""
        self.name = name
        self.role = role
        self.parent = parent

    def getRoleName(self) -> str:
        """Expose this fake node's accessible role name."""
        return self.role


class FakeApplication:
    """Expose one desktop application root with an optional registry instance ID."""

    def __init__(self, name: str, registry_id, states=()) -> None:
        """Retain the visible name, private registry ID, and native state set."""
        self.name = name
        self.registry_id = registry_id
        self.state = FakeState(states)

    @property
    def id(self):
        """Return the registry ID or model an AT-SPI bridge that cannot provide one."""
        if isinstance(self.registry_id, Exception):
            raise self.registry_id
        return self.registry_id

    def getState(self):
        """Return native root state for defunct filtering."""
        return self.state


class FakeDesktop:
    """Expose the registry desktop's ordered application roots for deduplication tests."""

    def __init__(self, children) -> None:
        """Retain the supplied application roots in their desktop order."""
        self.children = list(children)
        self.childCount = len(self.children)

    def getChildAtIndex(self, index: int):
        """Return one fake application root by its desktop index."""
        return self.children[index]


def record(name: str, role: str, enabled: bool = True) -> atspi_tool.NodeRecord:
    """Construct a compact enabled test record without a private desktop session."""
    return atspi_tool.NodeRecord(
        path="test",
        name=name,
        role=role,
        description="",
        enabled=enabled,
        focused=False,
        checked=False,
        pressed=False,
        selected=False,
        expanded=False,
        selected_item=None,
        text=None,
        value=None,
        minimum=None,
        maximum=None,
        actions=[],
        relations={},
    )


class AtspiToolTests(unittest.TestCase):
    """Cover native pressed state and fail-closed dropdown readback decisions."""

    def test_pressed_state_is_reported_for_native_toggles(self) -> None:
        """Reports `STATE_PRESSED` separately from checked state for GTK switches."""
        states = atspi_tool.safe_states(FakeNode([atspi_tool.pyatspi.STATE_PRESSED]))
        self.assertTrue(states["pressed"])
        self.assertFalse(states["checked"])

    def test_selector_readback_requires_the_dropdown_to_change(self) -> None:
        """Rejects an unchanged dropdown even when a popup list row claims selection."""
        before = atspi_tool.NodeRecord(
            path="selector",
            name="Pattern family",
            role="combo box",
            description="Pattern candidate: Custom pattern.",
            enabled=True,
            focused=False,
            checked=False,
            pressed=False,
            selected=False,
            expanded=False,
            selected_item=None,
            text=None,
            value=None,
            minimum=None,
            maximum=None,
            actions=[],
            relations={},
        )
        with patch.object(atspi_tool, "record_node", return_value=before):
            self.assertFalse(
                atspi_tool.selector_readback_changed(object(), before, "Straight Grid Circles")
            )

    def test_selector_readback_follows_a_rebuilt_dropdown(self) -> None:
        """Accepts the requested value from the unique replacement after a card rebuild."""
        before = record("Construction", "combo box")
        before.description = "Selected: Marks. Choose the construction output."
        replacement = record("Construction", "combo box")
        replacement.description = "Selected: Regions. Choose the construction output."
        arguments = type("Arguments", (), {})()
        with patch.object(atspi_tool, "record_node", return_value=before), patch.object(
            atspi_tool, "find_matches", return_value=[(object(), replacement)]
        ):
            self.assertTrue(
                atspi_tool.selector_readback_changed(object(), before, "Regions", arguments)
            )

    def test_selector_option_matching_stays_inside_the_opened_selector(self) -> None:
        """Finds an option label only through its selected selector's popup subtree."""
        selector = FakePopupNode("Pattern family", "combo box")
        popup = FakePopupNode("", "list", selector)
        item = FakePopupNode("", "list item", popup)
        label = FakePopupNode("Straight Grid Circles", "label", item)
        entries = [
            (selector, record("Pattern family", "combo box"), 0),
            (popup, record("", "list"), 1),
            (item, record("", "list item"), 2),
            (label, record("Straight Grid Circles", "label"), 3),
        ]
        with patch.object(atspi_tool, "walk", return_value=iter(entries)):
            self.assertEqual(
                atspi_tool.selector_option_matches(selector, "Straight Grid Circles", 8),
                [item],
            )

    def test_focus_event_matches_a_real_target_descendant(self) -> None:
        """Accepts a focused event source only when it belongs to the chosen target tree."""
        target = FakePopupNode("Pattern family", "combo box")
        child = FakePopupNode("", "toggle button", target)
        target_record = record("Pattern family", "combo box")
        entries = [
            (target, target_record, 0),
            (child, record("", "toggle button"), 1),
        ]
        with patch.object(atspi_tool, "walk", return_value=iter(entries)), patch.object(
            atspi_tool, "record_node", return_value=record("", "toggle button")
        ):
            self.assertIsNotNone(atspi_tool.focus_event_record(child, target, target_record))
        with patch.object(atspi_tool, "walk", return_value=iter(entries)):
            self.assertIsNone(atspi_tool.focus_event_record(FakePopupNode("", "toggle button"), target, target_record))

    def test_value_commit_requires_verified_focus_before_enter(self) -> None:
        """Rejects a scalar assignment when no real focus event or state was verified."""
        arguments = type("Arguments", (), {})()
        before = record("Opacity", "spin button")
        with patch.object(atspi_tool, "observe_focus", return_value=(None, None, None)), patch.object(
            atspi_tool, "press_loopback_key"
        ) as press:
            with self.assertRaisesRegex(SystemExit, "focus was not verified"):
                atspi_tool.commit_value_after_verified_focus(arguments, object(), before)
        press.assert_not_called()

    def test_value_commit_sends_enter_after_verified_focus(self) -> None:
        """Commits a scalar assignment only after the selected real control proves focus."""
        arguments = type("Arguments", (), {})()
        before = record("Opacity", "spin button")
        focused = record("Opacity", "spin button")
        with patch.object(atspi_tool, "observe_focus", return_value=(focused, "at-spi-focus-event", None)), patch.object(
            atspi_tool, "press_loopback_key"
        ) as press:
            record_after, method = atspi_tool.commit_value_after_verified_focus(arguments, object(), before)
        self.assertEqual(record_after, focused)
        self.assertEqual(method, "at-spi-focus-event")
        press.assert_called_once_with("enter")

    def test_text_commit_requires_focus_and_reresolves_a_rebuilt_entry(self) -> None:
        """Commits edited text through verified focus and reads back a rebuilt GTK entry."""
        arguments = type(
            "Arguments",
            (),
            {
                "actions": False,
                "set_value": None,
                "set_text": "0.27",
                "commit": True,
                "focus": False,
                "activate": None,
            },
        )()
        before = record("Y", "text")
        node = Mock()
        node.queryEditableText.return_value.setTextContents.return_value = True
        replacement = Mock()
        with patch.object(atspi_tool, "choose_match", return_value=(node, before)), patch.object(
            atspi_tool, "commit_value_after_verified_focus"
        ) as commit, patch.object(
            atspi_tool, "find_matches", return_value=[(replacement, record("Y", "text"))]
        ), patch.object(atspi_tool, "emit_change", return_value=0) as emit:
            self.assertEqual(atspi_tool.run_action(arguments), 0)
        node.queryEditableText.return_value.setTextContents.assert_called_once_with("0.27")
        commit.assert_called_once_with(arguments, node, before)
        emit.assert_called_once_with(replacement, before)

    def test_control_inventory_keeps_disabled_controls_and_omits_static_panels(self) -> None:
        """Lists a disabled button but excludes a non-interactive panel from semantic controls."""
        root = object()
        disabled_button = record("Apply Pattern", "button", enabled=False)
        panel = record("", "panel")
        arguments = type("Arguments", (), {
            "role": None,
            "ancestor": None,
            "depth": 4,
            "limit": 0,
            "subtree": None,
            "application": "Chromiator",
        })()
        entries = [(root, panel, 0), (object(), disabled_button, 1)]
        with patch.object(atspi_tool, "control_scope_roots", return_value=[(root, "application[0]")]), patch.object(
            atspi_tool, "walk", return_value=iter(entries)
        ), patch.object(atspi_tool, "is_defunct", return_value=False):
            self.assertEqual(atspi_tool.control_records(arguments), [disabled_button])

    def test_semantic_record_key_keeps_distinct_sibling_paths(self) -> None:
        """Keeps identical named sibling actions distinct for ancestor-scoped selection."""
        first = record("Delete Pattern", "button")
        second = record("Delete Pattern", "button")
        first.path = "application[0]/panel[4]/button[1]"
        second.path = "application[0]/panel[5]/button[1]"
        self.assertNotEqual(
            atspi_tool.semantic_record_key(first),
            atspi_tool.semantic_record_key(second),
        )

    def test_semantic_record_key_collapses_proven_equal_atspi_proxies(self) -> None:
        """Collapses repeated tree paths only when bus and object path prove proxy identity."""
        proxy = type("Proxy", (), {})()
        proxy.app = type("Application", (), {"bus_name": ":1.42"})()
        proxy.path = "/org/a11y/atspi/accessible/314"
        first = record("Saved Pattern", "toggle button")
        second = record("Saved Pattern", "toggle button")
        first.path = "application[0]/frame[1]/tablecell[19]"
        second.path = "application[0]/frame[2]/tablecell[19]"
        self.assertEqual(
            atspi_tool.semantic_record_key(first, proxy),
            atspi_tool.semantic_record_key(second, proxy),
        )

    def test_wait_state_decisions_use_native_enabled_and_checked_readback(self) -> None:
        """Accepts native disabled state and rejects an unrelated checked-state requirement."""
        disabled = record("Apply Pattern", "button", enabled=False)
        self.assertTrue(atspi_tool.wait_state_matches(disabled, "disabled"))
        self.assertFalse(atspi_tool.wait_state_matches(disabled, "checked"))

    def test_wait_rejects_ambiguous_present_targets(self) -> None:
        """Fails closed instead of silently waiting on an arbitrary repeated control."""
        arguments = type("Arguments", (), {"index": None})()
        matches = [(object(), record("Apply", "button")), (object(), record("Apply", "button"))]
        with patch.object(atspi_tool, "print_records"), self.assertRaisesRegex(
            SystemExit, "multiple nodes matched"
        ):
            atspi_tool.select_wait_match(arguments, matches)

    def test_application_roots_keep_same_name_distinct_registry_instances(self) -> None:
        """Retains same-named applications when AT-SPI assigns them different IDs."""
        first = FakeApplication("chromiator", 41)
        second = FakeApplication("chromiator", 42)
        desktop = FakeDesktop([first, second])
        with patch.object(atspi_tool.pyatspi.Registry, "getDesktop", return_value=desktop):
            self.assertEqual(atspi_tool.application_roots("Chromiator"), [first, second])

    def test_application_roots_collapse_only_same_registry_id(self) -> None:
        """Collapses duplicate proxies only when they expose the same AT-SPI registry ID."""
        first = FakeApplication("chromiator", 41)
        duplicate = FakeApplication("chromiator", 41)
        unreadable = FakeApplication("chromiator", RuntimeError("no registry ID"))
        desktop = FakeDesktop([first, duplicate, unreadable])
        with patch.object(atspi_tool.pyatspi.Registry, "getDesktop", return_value=desktop):
            self.assertEqual(atspi_tool.application_roots("Chromiator"), [first, unreadable])


if __name__ == "__main__":
    unittest.main()
