"""Small public D-Bus AT-SPI proxy for consistent accessible child snapshots.

libatspi's indexed-child fallback can ask GTK for a window while that window is
closing. GTK 4.22.5 then serializes a NULL object path and crashes. The AT-SPI
GetChildren method returns the child refs in one bus reply, so all indexing here
is local and never invokes Accessible.GetChildAtIndex in the application.
"""

from __future__ import annotations

import os

import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi, Gio, GLib


ACCESSIBLE = "org.a11y.atspi.Accessible"
PROPERTIES = "org.freedesktop.DBus.Properties"


def private_bus():
    """Connect only to the session's explicitly supplied accessibility bus."""
    address = os.environ.get("AT_SPI_BUS_ADDRESS")
    if not address:
        raise RuntimeError("AT_SPI_BUS_ADDRESS is required for the private AT-SPI snapshot")
    flags = (Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT
             | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION)
    return Gio.DBusConnection.new_for_address_sync(address, flags, None, None)


class StateSet:
    def __init__(self, words):
        self.words = words

    def contains(self, state):
        number = int(state)
        index = number // 32
        return index < len(self.words) and bool(self.words[index] & (1 << (number % 32)))


class Relation:
    def __init__(self, node, relation_type, refs):
        self.node = node
        self.relation_type = relation_type
        self.refs = refs

    def getRelationTypeName(self):
        return Atspi.RelationType(self.relation_type).value_nick.replace("-", " ")

    def getTarget(self):
        return [self.node.from_ref(ref) for ref in self.refs]


class Action:
    def __init__(self, node):
        self.node = node
        count = node.dbus_property("Action", "NActions")
        # GetActions contains localized labels; GetName supplies stable action IDs.
        self.names = [node.call("Action", "GetName", GLib.Variant("(i)", (i,)))[0]
                      for i in range(count)]

    @property
    def nActions(self):
        return len(self.names)

    def getName(self, index):
        return self.names[index]

    def doAction(self, index):
        return self.node.call("Action", "DoAction", GLib.Variant("(i)", (index,)))[0]


class Value:
    def __init__(self, node):
        self.node = node

    def _get(self, name):
        return self.node.dbus_property("Value", name)

    @property
    def currentValue(self):
        return self._get("CurrentValue")

    @currentValue.setter
    def currentValue(self, value):
        self.node.set_dbus_property("Value", "CurrentValue", GLib.Variant("d", float(value)))

    @property
    def minimumValue(self):
        return self._get("MinimumValue")

    @property
    def maximumValue(self):
        return self._get("MaximumValue")


class Text:
    def __init__(self, node):
        self.node = node

    @property
    def characterCount(self):
        return self.node.dbus_property("Text", "CharacterCount")

    def getText(self, start, end):
        return self.node.call("Text", "GetText", GLib.Variant("(ii)", (start, end)))[0]


class EditableText:
    def __init__(self, node):
        self.node = node

    def setTextContents(self, text):
        return self.node.call("EditableText", "SetTextContents", GLib.Variant("(s)", (text,)))[0]


class Selection:
    def __init__(self, node):
        self.node = node

    @property
    def nSelectedChildren(self):
        return self.node.call("Selection", "GetNSelectedChildren")[0]

    def getSelectedChild(self, index):
        ref = self.node.call("Selection", "GetSelectedChild", GLib.Variant("(i)", (index,)))[0]
        return self.node.from_ref(ref)

    def selectChild(self, index):
        return self.node.call("Selection", "SelectChild", GLib.Variant("(i)", (index,)))[0]


class Component:
    def __init__(self, node):
        self.node = node

    def grabFocus(self):
        return self.node.call("Component", "GrabFocus")[0]


class Accessible:
    """Duck-type the pyatspi methods used by the UI tool with public D-Bus calls."""

    def __init__(self, connection, bus_name, path):
        self.connection = connection
        self.bus_name = bus_name
        self.path = path
        self.app = self
        self._children = None

    def __eq__(self, other):
        return (isinstance(other, Accessible) and
                (self.bus_name, self.path) == (other.bus_name, other.path))

    def __hash__(self):
        return hash((self.bus_name, self.path))

    def call(self, interface, member, arguments=None):
        result = self.connection.call_sync(
            self.bus_name, self.path, f"org.a11y.atspi.{interface}", member,
            arguments, None, Gio.DBusCallFlags.NONE, 3000, None,
        )
        return result.unpack()

    def dbus_property(self, interface, name):
        result = self.connection.call_sync(
            self.bus_name, self.path, PROPERTIES, "Get",
            GLib.Variant("(ss)", (f"org.a11y.atspi.{interface}", name)),
            None, Gio.DBusCallFlags.NONE, 3000, None,
        )
        value = result.unpack()[0]
        return value.unpack() if isinstance(value, GLib.Variant) else value

    def set_dbus_property(self, interface, name, value):
        self.connection.call_sync(
            self.bus_name, self.path, PROPERTIES, "Set",
            GLib.Variant("(ssv)", (f"org.a11y.atspi.{interface}", name, value)),
            None, Gio.DBusCallFlags.NONE, 3000, None,
        )

    def from_ref(self, ref):
        bus_name, path = ref
        if not bus_name or path in ("", "/org/a11y/atspi/null"):
            raise RuntimeError("AT-SPI returned a null accessible reference")
        return Accessible(self.connection, bus_name, path)

    @property
    def name(self):
        return self.dbus_property("Accessible", "Name")

    @property
    def description(self):
        return self.dbus_property("Accessible", "Description")

    @property
    def parent(self):
        ref = self.dbus_property("Accessible", "Parent")
        return None if ref[1] == "/org/a11y/atspi/null" else self.from_ref(ref)

    @property
    def childCount(self):
        # Refresh for each traversal: a reused selector can gain popup children
        # after an action. Indexes still use this one atomic reply locally.
        self._children = tuple(self.call("Accessible", "GetChildren")[0])
        return len(self._children)

    def getChildAtIndex(self, index):
        if self._children is None:
            _ = self.childCount
        return self.from_ref(self._children[index])

    def getRoleName(self):
        # libatspi presents canonical role names; GTK's wire names differ
        # (for example Window vs Frame, Generic vs Panel).
        role = Atspi.Role(self.call("Accessible", "GetRole")[0])
        return Atspi.role_get_name(role)

    def getState(self):
        return StateSet(self.call("Accessible", "GetState")[0])

    def getRelationSet(self):
        return [Relation(self, kind, refs)
                for kind, refs in self.call("Accessible", "GetRelationSet")[0]]

    def queryAction(self):
        return Action(self)

    def queryValue(self):
        return Value(self)

    def queryText(self):
        return Text(self)

    def queryEditableText(self):
        return EditableText(self)

    def querySelection(self):
        return Selection(self)

    def queryComponent(self):
        return Component(self)
