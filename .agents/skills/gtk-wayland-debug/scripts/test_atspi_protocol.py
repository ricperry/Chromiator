#!/usr/bin/env python3
"""Focused public-protocol AT-SPI snapshot checks without a desktop session."""

from __future__ import annotations

import unittest

from gi.repository import GLib

from atspi_protocol import Accessible, StateSet


class FakeConnection:
    def __init__(self):
        self.calls = []

    def call_sync(self, destination, path, interface, member, arguments, *_rest):
        self.calls.append((destination, path, interface, member, arguments))
        if member == "GetChildren":
            return GLib.Variant("(a(so))", ([
                (destination, "/example/first"),
                (destination, "/example/second"),
            ],))
        if member == "Get" and arguments.unpack()[1] == "NActions":
            return GLib.Variant("(v)", (GLib.Variant("i", 2),))
        if member == "GetName":
            return GLib.Variant("(s)", (("click", "default.activate")[arguments.unpack()[0]],))
        if member == "DoAction":
            return GLib.Variant("(b)", (True,))
        if member == "Get" and arguments.unpack()[1] == "CurrentValue":
            return GLib.Variant("(v)", (GLib.Variant("d", 0.5),))
        if member == "Set":
            return GLib.Variant("()", ())
        raise AssertionError(f"unexpected AT-SPI call: {interface}.{member}")


class ProtocolTests(unittest.TestCase):
    def test_gtk_state_words_match_native_atspi_bits(self):
        """GTK sends two 32-bit words, as observed on a live enabled button."""
        states = StateSet([1 << 8 | 1 << 24, 0])
        self.assertTrue(states.contains(8))
        self.assertTrue(states.contains(24))
        self.assertFalse(states.contains(12))

    def test_children_are_one_snapshot_without_remote_index_calls(self):
        bus = FakeConnection()
        root = Accessible(bus, ":1.42", "/org/a11y/atspi/accessible/root")
        self.assertEqual(root.childCount, 2)
        self.assertEqual(root.getChildAtIndex(0).path, "/example/first")
        self.assertEqual(root.getChildAtIndex(1).path, "/example/second")
        self.assertEqual([call[3] for call in bus.calls], ["GetChildren"])

    def test_actions_keep_canonical_names_and_native_indices(self):
        bus = FakeConnection()
        node = Accessible(bus, ":1.42", "/example/first")
        action = node.queryAction()
        self.assertEqual(action.nActions, 2)
        self.assertEqual(action.getName(0), "click")
        self.assertTrue(action.doAction(0))
        self.assertEqual(bus.calls[-1][4].unpack(), (0,))

    def test_value_uses_readwrite_property_on_gtk_wire_interface(self):
        bus = FakeConnection()
        node = Accessible(bus, ":1.42", "/example/first")
        value = node.queryValue()
        self.assertEqual(value.currentValue, 0.5)
        value.currentValue = 0.75
        self.assertEqual(bus.calls[-1][3], "Set")
        interface, name, assigned = bus.calls[-1][4].unpack()
        self.assertEqual((interface, name), ("org.a11y.atspi.Value", "CurrentValue"))
        self.assertEqual(assigned, 0.75)

    def test_null_child_ref_is_rejected_instead_of_queried(self):
        node = Accessible(FakeConnection(), ":1.42", "/org/a11y/atspi/accessible/root")
        with self.assertRaisesRegex(RuntimeError, "null accessible reference"):
            node.from_ref((":1.42", "/org/a11y/atspi/null"))


if __name__ == "__main__":
    unittest.main()
