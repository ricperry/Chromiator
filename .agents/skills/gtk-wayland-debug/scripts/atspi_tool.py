#!/usr/bin/env python3
"""Query and operate real Chromiator GTK widgets through the private AT-SPI bus."""

from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
import json
from pathlib import Path
import subprocess
import sys
import threading
import time
from typing import Iterator

try:
    import pyatspi
except ImportError as error:
    raise SystemExit("Python AT-SPI bindings are missing; install python3-pyatspi") from error


@dataclass
class NodeRecord:
    """Store a compact, address-free snapshot of one accessible object."""

    path: str
    name: str
    role: str
    description: str
    enabled: bool
    focused: bool
    checked: bool
    pressed: bool
    selected: bool
    expanded: bool
    selected_item: str | None
    text: str | None
    value: float | None
    minimum: float | None
    maximum: float | None
    actions: list[str]
    relations: dict[str, list[str]]


INTERACTIVE_ROLES = frozenset({
    "button",
    "check box",
    "combo box",
    "link",
    "list box",
    "list item",
    "menu item",
    "page tab",
    "progress bar",
    "radio button",
    "scroll bar",
    "slider",
    "spin button",
    "switch",
    "text",
    "toggle button",
})

WAIT_STATES = {
    "enabled": lambda record: record.enabled,
    "disabled": lambda record: not record.enabled,
    "focused": lambda record: record.focused,
    "checked": lambda record: record.checked,
    "unchecked": lambda record: not record.checked,
    "pressed": lambda record: record.pressed,
    "unpressed": lambda record: not record.pressed,
    "selected": lambda record: record.selected,
    "unselected": lambda record: not record.selected,
    "expanded": lambda record: record.expanded,
    "collapsed": lambda record: not record.expanded,
}


def safe_text(getter) -> str:
    """Return an accessible string while tolerating transient GTK objects."""
    try:
        return str(getter() or "")
    except Exception:
        return "<unavailable>"


def safe_value(node) -> tuple[float | None, float | None, float | None]:
    """Read the optional AT-SPI Value interface without changing a widget."""
    try:
        value = node.queryValue()
        return float(value.currentValue), float(value.minimumValue), float(value.maximumValue)
    except Exception:
        return None, None, None


def safe_node_text(node) -> str | None:
    """Read optional editable or static text from a live accessible object."""
    try:
        text = node.queryText()
        return str(text.getText(0, text.characterCount))
    except Exception:
        return None


def safe_actions(node) -> list[str]:
    """Return available action names without invoking them."""
    try:
        action = node.queryAction()
        return [str(action.getName(index)) for index in range(action.nActions)]
    except Exception:
        return []


def safe_states(node) -> dict[str, bool]:
    """Read the semantic state flags needed to operate ordinary controls."""
    try:
        state = node.getState()
        return {
            "enabled": state.contains(pyatspi.STATE_ENABLED) or state.contains(pyatspi.STATE_SENSITIVE),
            "focused": state.contains(pyatspi.STATE_FOCUSED),
            "checked": state.contains(pyatspi.STATE_CHECKED),
            "pressed": state.contains(pyatspi.STATE_PRESSED),
            "selected": state.contains(pyatspi.STATE_SELECTED),
            "expanded": state.contains(pyatspi.STATE_EXPANDED),
        }
    except Exception:
        return {
            key: False
            for key in ("enabled", "focused", "checked", "pressed", "selected", "expanded")
        }


def safe_relations(node) -> dict[str, list[str]]:
    """Read useful AT-SPI relation targets by name instead of object address."""
    try:
        relation_set = node.getRelationSet()
    except Exception:
        return {}
    result: dict[str, list[str]] = {}
    for relation in relation_set:
        try:
            targets = [safe_text(lambda target=target: target.name) for target in relation.getTarget()]
            if targets:
                result[str(relation.getRelationTypeName())] = targets
        except Exception:
            continue
    return result


def safe_selected_item(node) -> str | None:
    """Read native Selection or the collapsed GTK dropdown's single displayed label."""
    try:
        selection = node.querySelection()
        if selection.nSelectedChildren:
            return safe_text(lambda: selection.getSelectedChild(0).name)
    except Exception:
        pass
    # GTK DropDown exposes its selected label under the toggle, without Selection.
    # Inspect only that button, never popup options or an arbitrary descendant list.
    try:
        if node.getRoleName() == "combo box" and node.childCount:
            button = node.getChildAtIndex(0)
            if button.getRoleName() == "toggle button":
                labels = []
                pending = [(button, 0)]
                while pending:
                    child, depth = pending.pop()
                    role = child.getRoleName()
                    if role == "label" and child.name:
                        labels.append(child.name)
                    elif depth < 10 and role not in {"list", "menu", "list item"}:
                        pending.extend((child.getChildAtIndex(i), depth + 1)
                                       for i in range(child.childCount))
                if len(labels) == 1:
                    return labels[0]
    except Exception:
        pass
    return None


def record_node(node, path: str) -> NodeRecord:
    """Snapshot compact semantic data for query results and action readback."""
    value, minimum, maximum = safe_value(node)
    return NodeRecord(
        path=path,
        name=safe_text(lambda: node.name),
        role=safe_text(node.getRoleName),
        description=safe_text(lambda: node.description),
        text=safe_node_text(node),
        value=value,
        minimum=minimum,
        maximum=maximum,
        actions=safe_actions(node),
        relations=safe_relations(node),
        selected_item=safe_selected_item(node),
        **safe_states(node),
    )


def walk(node, path: str, depth: int = 0, max_depth: int = 30) -> Iterator[tuple[object, NodeRecord, int]]:
    """Traverse a bounded subtree with semantic paths and stale-node tolerance."""
    record = record_node(node, path)
    yield node, record, depth
    if depth >= max_depth:
        return
    try:
        child_count = node.childCount
    except Exception:
        return
    for index in range(child_count):
        try:
            child = node.getChildAtIndex(index)
            if child is not None:
                child_record = record_node(child, "")
                segment = f"{child_record.role}:{child_record.name or '<unnamed>'}[{index}]"
                yield from walk(child, f"{path}/{segment}", depth + 1, max_depth)
        except Exception:
            continue


def application_roots(application_query: str | None) -> list[object]:
    """Select non-defunct private app roots without merging distinct registry instances."""
    root = pyatspi.Registry.getDesktop(0)
    if not application_query:
        return [root]
    query = application_query.casefold()
    roots = []
    registry_ids = set()
    for index in range(root.childCount):
        try:
            candidate = root.getChildAtIndex(index)
            state = candidate.getState()
            if state.contains(pyatspi.STATE_DEFUNCT):
                continue
            name = safe_text(lambda: candidate.name)
            if query not in name.casefold():
                continue
            try:
                registry_id = candidate.id
                hash(registry_id)
            except Exception:
                registry_id = None
            if registry_id is not None:
                if registry_id in registry_ids:
                    continue
                registry_ids.add(registry_id)
            roots.append(candidate)
        except Exception:
            continue
    return roots


def has_named_ancestor(node, query: str) -> bool:
    """Match a node by a stable named accessible ancestor, never screen position."""
    query = query.casefold()
    current = node
    for _ in range(32):
        try:
            current = current.parent
        except Exception:
            return False
        if current is None:
            return False
        if query in safe_text(lambda: current.name).casefold():
            return True
    return False


def is_defunct(node) -> bool:
    """Reject stale AT-SPI proxies before they can duplicate a live semantic match."""
    try:
        return node.getState().contains(pyatspi.STATE_DEFUNCT)
    except Exception:
        return True


def semantic_record_key(record: NodeRecord, node=None) -> tuple[str, str, str, str]:
    """Deduplicate only a proven AT-SPI bus/object proxy, retaining unknown siblings."""
    if node is not None:
        try:
            bus_name = str(node.app.bus_name)
            object_path = str(node.path)
            if bus_name and object_path:
                return "proxy", bus_name, object_path, ""
        except Exception:
            pass
    return "record", record.role, record.name, record.path


def is_accessibility_control(record: NodeRecord) -> bool:
    """Keep control inventory output to real operable or state-bearing GTK nodes."""
    return (
        record.role in INTERACTIVE_ROLES
        or (bool(record.actions) and record.role not in {"label", "static"})
        or record.value is not None
        or record.checked
        or record.pressed
        or record.selected
        or record.expanded
    )


def wait_state_matches(record: NodeRecord, state: str | None) -> bool:
    """Evaluate one explicit native wait-state requirement against a compact record."""
    return state is None or WAIT_STATES[state](record)


def find_matches(arguments) -> list[tuple[object, NodeRecord]]:
    """Find live nodes by semantic name, optional role, and optional ancestor."""
    query = arguments.query.casefold()
    role_query = arguments.role.casefold() if arguments.role else None
    matches = []
    seen = set()
    for root_index, root in enumerate(application_roots(arguments.application)):
        for node, record, _depth in walk(root, f"application[{root_index}]", max_depth=arguments.depth):
            if is_defunct(node):
                continue
            name_match = record.name.casefold() == query if arguments.exact else query in record.name.casefold()
            role_match = role_query is None or role_query == record.role.casefold()
            ancestor_match = not arguments.ancestor or has_named_ancestor(node, arguments.ancestor)
            if name_match and role_match and ancestor_match:
                key = semantic_record_key(record, node)
                if key in seen:
                    continue
                seen.add(key)
                matches.append((node, record))
                if getattr(arguments, "limit", 0) and len(matches) >= arguments.limit:
                    return matches
    return matches


def print_records(records: list[NodeRecord], as_json: bool) -> None:
    """Render concise JSON or text records suitable for small agent queries."""
    if as_json:
        print(json.dumps([asdict(record) for record in records], indent=2, sort_keys=True))
        return
    for index, record in enumerate(records):
        details = [f"[{index}]", record.role or "unknown", repr(record.name), f"path={record.path}"]
        details.append("enabled" if record.enabled else "disabled")
        details.extend(
            key
            for key in ("focused", "checked", "pressed", "selected", "expanded")
            if getattr(record, key)
        )
        if record.value is not None:
            details.extend((f"value={record.value:g}", f"range={record.minimum:g}..{record.maximum:g}"))
        if record.text is not None and record.text != record.name:
            details.append(f"text={record.text!r}")
        if record.actions:
            details.append(f"actions={','.join(record.actions)}")
        if record.relations:
            details.append("relations=" + json.dumps(record.relations, sort_keys=True))
        if record.selected_item:
            details.append(f"selected_item={record.selected_item!r}")
        print(" ".join(details))


def choose_match(arguments) -> tuple[object, NodeRecord]:
    """Choose exactly one node or report semantic candidates for disambiguation."""
    matches = find_matches(arguments)
    if not matches:
        raise SystemExit("no accessible node matched")
    if arguments.index is None and len(matches) != 1:
        print_records([record for _node, record in matches], False)
        raise SystemExit("multiple nodes matched; repeat with --index N or --ancestor NAME")
    index = arguments.index or 0
    if index < 0 or index >= len(matches):
        raise SystemExit(f"match index {index} is out of range")
    return matches[index]


def invoke(node, requested: str | None = None) -> None:
    """Invoke a requested or conventional live AT-SPI action on one widget."""
    action = node.queryAction()
    available = [str(action.getName(index)) for index in range(action.nActions)]
    selected = requested or next((name for name in ("activate", "click", "press", "open", "toggle") if name in available), None)
    if not selected or selected not in available:
        descendants = [
            (candidate, record)
            for candidate, record, _depth in walk(node, "action target", max_depth=4)
            if candidate != node and (requested in record.actions if requested else bool(record.actions))
        ]
        if len(descendants) != 1:
            candidates = [f"{record.role} {record.name!r}: {record.actions}" for _node, record in descendants]
            raise SystemExit(f"requested action is unavailable; actions={available}; descendants={candidates}")
        invoke(descendants[0][0], requested)
        return
    if not action.doAction(available.index(selected)):
        raise SystemExit(f"AT-SPI action was rejected: {selected}")


def emit_change(node, before: NodeRecord, after_node=None) -> int:
    """Emit before/after JSON after a real-widget operation and short GTK update turn."""
    time.sleep(0.05)
    current = after_node or node
    print(json.dumps({"before": asdict(before), "after": asdict(record_node(current, before.path))}, indent=2, sort_keys=True))
    return 0


def emit_selection(selector, before: NodeRecord, option, option_name: str, method: str) -> int:
    """Emit selector and selected live-list-item state after an AT-SPI selection."""
    time.sleep(0.05)
    print(json.dumps({
        "before": asdict(before),
        "after": asdict(record_node(selector, before.path)),
        "method": method,
        "selected_option": option_name,
        "option_state": asdict(record_node(option, "live option")),
    }, indent=2, sort_keys=True))
    return 0


def press_loopback_key(key: str) -> None:
    """Send one normal key through the private loopback session without coordinates."""
    helper = Path(__file__).with_name("vnc_client.py")
    subprocess.run([sys.executable, str(helper), "key", key], check=True, capture_output=True, text=True)


def selector_record_matches_option(after: NodeRecord, before: NodeRecord, option_name: str) -> bool:
    """Report whether one selector record proves the requested option was applied."""
    description_changed = (
        after.description != before.description
        and option_name.casefold() in after.description.casefold()
    )
    selected_item_matches = (
        after.selected_item is not None
        and after.selected_item.casefold() == option_name.casefold()
    )
    return description_changed or selected_item_matches


def selector_readback_changed(selector, before: NodeRecord, option_name: str, arguments=None) -> bool:
    """Confirm dropdown readback, re-resolving a selector replaced by a GTK card rebuild."""
    after = record_node(selector, before.path)
    if selector_record_matches_option(after, before, option_name):
        return True
    if arguments is None:
        return False
    replacements = find_matches(arguments)
    return len(replacements) == 1 and selector_record_matches_option(
        replacements[0][1], before, option_name
    )


def ancestors(node) -> Iterator[object]:
    """Yield bounded semantic ancestors while tolerating a GTK popup closing."""
    current = node
    for _ in range(16):
        try:
            current = current.parent
        except Exception:
            return
        if current is None:
            return
        yield current


def list_box_index(list_item) -> tuple[object, int] | None:
    """Find a popup list box and the option's ordinal among its real list items."""
    for owner in ancestors(list_item):
        actions = safe_actions(owner)
        if safe_text(owner.getRoleName) != "list" and "default.activate" not in actions:
            continue
        items = [
            candidate
            for candidate, record, _depth in walk(owner, "popup list", max_depth=4)
            if record.role == "list item"
        ]
        for index, candidate in enumerate(items):
            if candidate == list_item:
                return owner, index
    return None


def list_item_ancestor(node, selector_nodes: list[object]) -> object | None:
    """Return the local popup list item that owns one named option descendant."""
    current = node
    for _ in range(6):
        if safe_text(current.getRoleName) == "list item" and any(current == candidate for candidate in selector_nodes):
            return current
        try:
            current = current.parent
        except Exception:
            return None
        if current is None:
            return None
    return None


def selector_option_matches(selector, option_name: str, max_depth: int) -> list[object]:
    """Find enabled option rows only inside the opened selector-owned popup subtree."""
    entries = list(walk(selector, "selector popup", max_depth=max_depth))
    selector_nodes = [node for node, _record, _depth in entries]
    matches = []
    seen = []
    for node, record, _depth in entries:
        if not record.enabled or record.name.casefold() != option_name.casefold():
            continue
        list_item = list_item_ancestor(node, selector_nodes)
        if list_item is not None and not any(list_item == previous for previous in seen):
            seen.append(list_item)
            matches.append(list_item)
    return matches


def focus_event_record(source, target, target_record: NodeRecord) -> NodeRecord | None:
    """Accept one focused event only when its source is the target or a real descendant."""
    try:
        if source == target:
            return record_node(source, target_record.path)
    except Exception:
        return None
    for candidate, record, _depth in walk(target, target_record.path, max_depth=6):
        try:
            if source == candidate:
                return record_node(source, record.path)
        except Exception:
            continue
    return None


def focused_interactive_node(node) -> NodeRecord | None:
    """Return a focused real control at or below one semantic target node."""
    interactive_roles = {
        "check box",
        "combo box",
        "push button",
        "radio button",
        "spin button",
        "switch",
        "text",
        "toggle button",
    }
    for candidate, record, _depth in walk(node, "focus target", max_depth=6):
        if record.focused and (
            record.role in interactive_roles or record.actions or safe_node_text(candidate) is not None
        ):
            return record
    return None


def poll_focused_match(arguments) -> NodeRecord | None:
    """Re-find a semantic target and return its focused interactive node after one Tab turn."""
    for node, _record in find_matches(arguments):
        focused = focused_interactive_node(node)
        if focused is not None:
            return focused
    return None


def emit_focus(before: NodeRecord, node, focused_record: NodeRecord, method: str) -> int:
    """Emit verified focus state without treating a requested focus as proof of focus."""
    print(json.dumps({
        "before": asdict(before),
        "after": asdict(record_node(node, before.path)),
        "focus_method": method,
        "focused_target": asdict(focused_record),
    }, indent=2, sort_keys=True))
    return 0


def observe_focus(arguments, node, before: NodeRecord) -> tuple[NodeRecord | None, str | None, NodeRecord | None]:
    """Observe standard focus events while bounded component and keyboard attempts run."""
    steps = getattr(arguments, "steps", 80)
    result: dict[str, NodeRecord | str | None] = {
        "event": None,
        "polled": None,
        "method": None,
    }
    completed = threading.Event()
    lock = threading.Lock()

    def on_focus(event) -> None:
        """Record one true focused event whose source belongs to the chosen target."""
        if not getattr(event, "detail1", False):
            return
        record = focus_event_record(event.source, node, before)
        if record is None:
            return
        with lock:
            result["event"] = record
            result["method"] = "at-spi-focus-event"
        completed.set()
        pyatspi.Registry.stop()

    def attempts() -> None:
        """Perform bounded direct and Tab focus requests while the registry pumps events."""
        try:
            try:
                focused = node.queryComponent().grabFocus()
            except Exception:
                focused = False
            if focused:
                time.sleep(0.05)
                focused_record = poll_focused_match(arguments)
                if focused_record is not None:
                    with lock:
                        result["polled"] = focused_record
                        result["method"] = "at-spi-component-state"
                    return
            for key in ("tab", "shift-tab"):
                for _ in range(steps):
                    if completed.is_set():
                        return
                    press_loopback_key(key)
                    time.sleep(0.03)
                    focused_record = poll_focused_match(arguments)
                    if focused_record is not None:
                        with lock:
                            result["polled"] = focused_record
                            result["method"] = f"loopback-{key}-state"
                        return
        finally:
            completed.set()
            pyatspi.Registry.stop()

    timeout_seconds = max(3.0, 1.0 + steps * 0.16 * 2)
    timeout = threading.Timer(timeout_seconds, pyatspi.Registry.stop)
    pyatspi.Registry.registerEventListener(on_focus, "object:state-changed:focused")
    worker = threading.Thread(target=attempts, daemon=True)
    try:
        timeout.start()
        worker.start()
        pyatspi.Registry.start()
    finally:
        timeout.cancel()
        pyatspi.Registry.deregisterEventListener(on_focus, "object:state-changed:focused")
        worker.join(timeout=1.0)
    with lock:
        return result["event"], result["method"], result["polled"]


def commit_value_after_verified_focus(arguments, node, before: NodeRecord) -> tuple[NodeRecord, str]:
    """Send Enter only after the real numeric widget proves semantic focus ownership."""
    if not hasattr(arguments, "steps"):
        arguments.steps = 80
    event_record, method, polled_record = observe_focus(arguments, node, before)
    focused_record = event_record or polled_record
    if focused_record is None:
        raise SystemExit(
            f"value assignment for {before.name!r} was not committed because focus was not verified"
        )
    press_loopback_key("enter")
    return focused_record, method or "at-spi-state"


def run_tree(arguments) -> int:
    """Print one bounded application tree or a named live subtree."""
    if arguments.subtree:
        arguments.query = arguments.subtree
        node, record = choose_match(arguments)
        entries = list(walk(node, record.path, max_depth=arguments.depth))
    else:
        entries = [entry for root_index, root in enumerate(application_roots(arguments.application)) for entry in walk(root, f"application[{root_index}]", max_depth=arguments.depth)]
    if arguments.json:
        print_records([record for _node, record, _depth in entries], True)
    else:
        for _node, record, depth in entries:
            suffix = f" value={record.value:g}" if record.value is not None else ""
            print("  " * depth + f"{record.role} {record.name!r}{suffix}")
    return 0 if entries else 3


def run_find(arguments) -> int:
    """Print all narrowly selected live semantic matches."""
    matches = find_matches(arguments)
    print_records([record for _node, record in matches], arguments.json)
    return 0 if matches else 3


def control_scope_roots(arguments) -> list[tuple[object, str]]:
    """Resolve an optional named subtree before listing its interactive controls."""
    if not arguments.subtree:
        return [
            (root, f"application[{root_index}]")
            for root_index, root in enumerate(application_roots(arguments.application))
        ]
    selector = argparse.Namespace(
        application=arguments.application,
        query=arguments.subtree,
        role=arguments.subtree_role,
        exact=True,
        ancestor=None,
        depth=30,
        limit=0,
        index=None,
    )
    matches = find_matches(selector)
    if not matches:
        raise SystemExit("no accessible node matched")
    if len(matches) == 1:
        node, record = matches[0]
        return [(node, record.path)]
    interactive = [(node, record) for node, record in matches if is_accessibility_control(record)]
    if interactive:
        shallowest_depth = min(record.path.count("/") for _node, record in interactive)
        shallowest = [
            (node, record)
            for node, record in interactive
            if record.path.count("/") == shallowest_depth
        ]
        if len(shallowest) == 1:
            node, record = shallowest[0]
            return [(node, record.path)]
    print_records([record for _node, record in matches], False)
    raise SystemExit("subtree is ambiguous; repeat with --subtree-role ROLE")


def control_records(arguments) -> list[NodeRecord]:
    """Collect a deduplicated semantic inventory without hiding disabled real controls."""
    role_query = arguments.role.casefold() if arguments.role else None
    records = []
    seen = set()
    for root, path in control_scope_roots(arguments):
        for node, record, _depth in walk(root, path, max_depth=arguments.depth):
            if is_defunct(node) or not is_accessibility_control(record):
                continue
            if role_query is not None and record.role.casefold() != role_query:
                continue
            if arguments.ancestor and not has_named_ancestor(node, arguments.ancestor):
                continue
            key = semantic_record_key(record, node)
            if key in seen:
                continue
            seen.add(key)
            records.append(record)
            if arguments.limit and len(records) >= arguments.limit:
                return records
    return records


def run_controls(arguments) -> int:
    """Print only interactive or state-bearing GTK controls under an optional semantic scope."""
    records = control_records(arguments)
    print_records(records, arguments.json)
    return 0 if records else 3


def select_wait_match(arguments, matches: list[tuple[object, NodeRecord]]) -> NodeRecord:
    """Choose one wait target and reject ambiguous live controls before polling state."""
    if arguments.index is None and len(matches) != 1:
        print_records([record for _node, record in matches], False)
        raise SystemExit("multiple nodes matched; repeat with --index N or --ancestor NAME")
    index = arguments.index or 0
    if index < 0 or index >= len(matches):
        raise SystemExit(f"match index {index} is out of range")
    return matches[index][1]


def emit_wait(record: NodeRecord | None, arguments) -> int:
    """Emit a compact present or absent synchronization result without an object address."""
    if arguments.json:
        print(json.dumps({
            "present": record is not None,
            "state": arguments.state,
            "record": asdict(record) if record is not None else None,
        }, indent=2, sort_keys=True))
    elif record is None:
        print(f"absent {arguments.query!r}")
    else:
        print_records([record], False)
    return 0


def run_wait(arguments) -> int:
    """Wait for one semantic node to appear, disappear, or expose an explicit native state."""
    deadline = time.monotonic() + arguments.timeout
    while True:
        matches = find_matches(arguments)
        if arguments.absent:
            if not matches:
                return emit_wait(None, arguments)
        elif matches:
            record = select_wait_match(arguments, matches)
            if wait_state_matches(record, arguments.state):
                return emit_wait(record, arguments)
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            if arguments.absent:
                raise SystemExit(f"timed out waiting for {arguments.query!r} to be absent")
            state_suffix = f" with state {arguments.state}" if arguments.state else ""
            raise SystemExit(f"timed out waiting for {arguments.query!r} to be present{state_suffix}")
        time.sleep(min(arguments.interval / 1000.0, remaining))


def run_action(arguments) -> int:
    """Perform one focus, action, scalar-value, or editable-text operation."""
    if arguments.commit and arguments.set_text is None:
        raise SystemExit("--commit is valid only with --set-text")
    node, before = choose_match(arguments)
    if arguments.actions:
        print_records([before], arguments.json)
        return 0
    if arguments.set_value is not None:
        if before.minimum is None or before.maximum is None:
            raise SystemExit("matched node does not expose the AT-SPI Value interface")
        if not before.minimum <= arguments.set_value <= before.maximum:
            raise SystemExit(f"value {arguments.set_value:g} is outside {before.minimum:g}..{before.maximum:g}")
        node.queryValue().currentValue = arguments.set_value
        # Native scrollbars apply Value changes immediately and cannot take keyboard focus.
        # Tab traversal here moves the viewport again and corrupts the requested readback.
        if before.role != "scroll bar":
            commit_value_after_verified_focus(arguments, node, before)
        node, _after = choose_match(arguments)
    elif arguments.set_text is not None:
        if not node.queryEditableText().setTextContents(arguments.set_text):
            raise SystemExit("AT-SPI editable-text request was rejected")
        if arguments.commit:
            commit_value_after_verified_focus(arguments, node, before)
            replacements = find_matches(arguments)
            if len(replacements) == 1:
                node = replacements[0][0]
    elif arguments.focus:
        event_record, method, polled_record = observe_focus(arguments, node, before)
        if event_record is not None:
            return emit_focus(before, node, event_record, method or "at-spi-focus-event")
        if polled_record is not None:
            return emit_focus(before, node, polled_record, method or "at-spi-state")
        raise SystemExit(
            f"could not observe focus for {before.name!r} through the standard AT-SPI "
            "focus event or bounded component/Tab state traversal"
        )
    else:
        invoke(node, arguments.activate if arguments.activate is not True else None)
    return emit_change(node, before)


def run_select(arguments) -> int:
    """Open a real GTK dropdown and activate its live visible option through AT-SPI."""
    selector, before = choose_match(arguments)
    if not before.expanded:
        invoke(selector, "click" if "click" in before.actions else None)
    options = []
    for _ in range(20):
        options = selector_option_matches(selector, arguments.option, arguments.depth)
        if options:
            break
        time.sleep(0.05)
    if len(options) != 1:
        if len(options) > 1:
            raise SystemExit(f"dropdown option is ambiguous: {arguments.option}")
        raise SystemExit(f"no live dropdown option matched: {arguments.option}")
    node = options[0]
    list_item = None
    for _ in range(5):
        if safe_text(node.getRoleName) == "list item":
            list_item = node
            break
        node = getattr(node, "parent", None)
        if node is None:
            break
    if list_item is None:
        raise SystemExit(f"dropdown option has no live list item ancestor: {arguments.option}")
    list_box = list_box_index(list_item)
    if list_box is None:
        raise SystemExit(f"dropdown option has no live list box ancestor: {arguments.option}")
    owner, index = list_box
    list_item_selected = False
    try:
        owner.querySelection().selectChild(index)
        list_item_selected = record_node(list_item, "live option").selected
        if list_item_selected and "default.activate" in safe_actions(owner):
            invoke(owner, "default.activate")
            time.sleep(0.1)
            if selector_readback_changed(selector, before, arguments.option, arguments):
                return emit_selection(selector, before, list_item, arguments.option, "selection-default.activate")
    except Exception:
        pass
    try:
        helper = Path(__file__).with_name("vnc_client.py")
        subprocess.run(
            [sys.executable, str(helper), "dropdown", before.name, str(index),
             "--application", arguments.application],
            check=True, capture_output=True, text=True, timeout=20,
        )
        for _ in range(20):
            time.sleep(0.05)
            if selector_readback_changed(selector, before, arguments.option, arguments):
                return emit_selection(selector, before, list_item, arguments.option,
                                      "persistent-loopback-home-down-enter")
    except Exception as error:
        detail = getattr(error, "stderr", None) or str(error)
        raise SystemExit(f"dropdown loopback keyboard input failed: {detail}") from None
    raise SystemExit(
        "dropdown option could not be activated: "
        f"{arguments.option} (list index {index}; selector readback "
        f"{record_node(selector, before.path).description!r})"
    )


def add_query_arguments(parser: argparse.ArgumentParser) -> None:
    """Add stable semantic selector flags without object addresses or coordinates."""
    parser.add_argument("query")
    parser.add_argument("--application", default="Chromiator")
    parser.add_argument("--role")
    parser.add_argument("--exact", action="store_true")
    parser.add_argument("--ancestor", help="require a named accessible ancestor")
    parser.add_argument("--depth", type=int, default=30)
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--index", type=int)


def build_parser() -> argparse.ArgumentParser:
    """Build the compact private-bus tree, query, and operation CLI."""
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("check")
    tree = commands.add_parser("tree")
    tree.add_argument("--application", default="Chromiator")
    tree.add_argument("--subtree")
    tree.add_argument("--role")
    tree.add_argument("--exact", action="store_true")
    tree.add_argument("--ancestor")
    tree.add_argument("--index", type=int)
    tree.add_argument("--depth", type=int, default=12)
    tree.add_argument("--json", action="store_true")
    for command in ("find", "inspect", "focus", "activate", "set", "toggle", "type", "select"):
        add_query_arguments(commands.add_parser(command))
    controls = commands.add_parser("controls", help="list interactive or state-bearing GTK controls")
    controls.add_argument("--application", default="Chromiator")
    controls.add_argument("--subtree", help="scope to one exact named accessible subtree")
    controls.add_argument("--subtree-role", help="disambiguate the named subtree by accessible role")
    controls.add_argument("--ancestor", help="require a named accessible ancestor")
    controls.add_argument("--role")
    controls.add_argument("--depth", type=int, default=12)
    controls.add_argument("--limit", type=int, default=0)
    controls.add_argument("--json", action="store_true")
    wait = commands.add_parser("wait", help="wait for a semantic node or native state")
    add_query_arguments(wait)
    wait.add_argument("--present", dest="absent", action="store_false", default=False)
    wait.add_argument("--absent", action="store_true")
    wait.add_argument("--state", choices=sorted(WAIT_STATES))
    wait.add_argument("--timeout", type=float, default=3.0)
    wait.add_argument("--interval", type=int, default=50)
    commands.choices["focus"].add_argument(
        "--steps",
        type=int,
        default=24,
        help="maximum Tab and Shift+Tab turns for the coordinate-free focus fallback",
    )
    commands.choices["set"].add_argument("value", type=float)
    commands.choices["type"].add_argument("text")
    commands.choices["type"].add_argument("--commit", action="store_true")
    commands.choices["select"].add_argument("option")
    action = commands.add_parser("action")
    add_query_arguments(action)
    group = action.add_mutually_exclusive_group(required=True)
    group.add_argument("--activate", nargs="?", const=True, metavar="ACTION")
    group.add_argument("--set-value", type=float)
    group.add_argument("--set-text")
    group.add_argument("--focus", action="store_true")
    group.add_argument("--actions", action="store_true")
    action.add_argument("--commit", action="store_true")
    return parser


def main() -> int:
    """Dispatch one semantic operation without normal-widget coordinate input."""
    arguments = build_parser().parse_args()
    arguments.commit = getattr(arguments, "commit", False)
    if arguments.command == "wait":
        if arguments.absent and arguments.state:
            raise SystemExit("--state cannot be combined with --absent")
        if not 0 < arguments.timeout <= 30:
            raise SystemExit("--timeout must be greater than zero and at most 30 seconds")
        if not 10 <= arguments.interval <= 1000:
            raise SystemExit("--interval must be between 10 and 1000 milliseconds")
    if arguments.command == "check":
        return 0
    if arguments.command == "tree":
        return run_tree(arguments)
    if arguments.command == "find":
        return run_find(arguments)
    if arguments.command == "controls":
        return run_controls(arguments)
    if arguments.command == "wait":
        return run_wait(arguments)
    if arguments.command == "inspect":
        _node, record = choose_match(arguments)
        print_records([record], True)
        return 0
    if arguments.command in ("focus", "activate", "set", "type"):
        arguments.focus = arguments.command == "focus"
        arguments.activate = True if arguments.command == "activate" else None
        arguments.set_value = arguments.value if arguments.command == "set" else None
        arguments.set_text = arguments.text if arguments.command == "type" else None
        arguments.actions = False
        return run_action(arguments)
    if arguments.command == "toggle":
        node, before = choose_match(arguments)
        invoke(node, "toggle" if "toggle" in before.actions else None)
        time.sleep(0.05)
        after_node, _after = choose_match(arguments)
        return emit_change(node, before, after_node)
    if arguments.command == "select":
        return run_select(arguments)
    if arguments.command == "action":
        return run_action(arguments)
    raise AssertionError(f"unhandled command: {arguments.command}")


if __name__ == "__main__":
    sys.exit(main())
