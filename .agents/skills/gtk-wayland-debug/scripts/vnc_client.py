#!/usr/bin/env python3
"""Drive loopback-only WayVNC and terminate its Twisted helper cleanly."""

from __future__ import annotations

import argparse
import importlib.metadata
import json
import os
from pathlib import Path
import subprocess
import sys


def load_api():
    """Import VNCDoTool with a concise installation error."""
    try:
        from vncdotool import api
    except ImportError as error:
        raise SystemExit(
            f"VNCDoTool import failed: {error}. Install vncdotool."
        ) from error
    return api


def parser() -> argparse.ArgumentParser:
    """Build the private VNC helper command parser."""
    command_parser = argparse.ArgumentParser()
    subparsers = command_parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("check")
    subparsers.add_parser("version")

    click_parser = subparsers.add_parser("click")
    click_parser.add_argument("x", type=int)
    click_parser.add_argument("y", type=int)
    click_parser.add_argument("button", type=int, nargs="?", default=1)

    move_parser = subparsers.add_parser("move")
    move_parser.add_argument("x", type=int)
    move_parser.add_argument("y", type=int)

    drag_parser = subparsers.add_parser("drag")
    drag_parser.add_argument("start_x", type=int)
    drag_parser.add_argument("start_y", type=int)
    drag_parser.add_argument("end_x", type=int)
    drag_parser.add_argument("end_y", type=int)
    drag_parser.add_argument("button", type=int, nargs="?", default=1)
    drag_parser.add_argument("steps", type=int, nargs="?", default=8)

    type_parser = subparsers.add_parser("type")
    type_parser.add_argument("text")

    key_parser = subparsers.add_parser("key")
    key_parser.add_argument("key")
    dropdown_parser = subparsers.add_parser("dropdown")
    dropdown_parser.add_argument("query")
    dropdown_parser.add_argument("index", type=int)
    dropdown_parser.add_argument("--application", default="Chromiator")
    return command_parser


def main() -> int:
    """Execute one unauthenticated VNC input primitive against loopback."""
    arguments = parser().parse_args()
    api = load_api()
    if arguments.command == "check":
        return 0
    if arguments.command == "version":
        print(f"vncdotool={importlib.metadata.version('vncdotool')}")
        return 0

    server = os.environ.get("CHROMIATOR_VNC_SERVER", "127.0.0.1::5901")
    if not (server.startswith("127.0.0.1::") or server.startswith("localhost::")):
        raise SystemExit(f"refusing non-loopback VNC server: {server}")
    try:
        with api.connect(server, timeout=15) as client:
            if arguments.command == "move":
                if arguments.x < 0 or arguments.y < 0:
                    raise SystemExit("mouse coordinates must be non-negative")
                client.mouseMove(arguments.x, arguments.y)
                client.pause(0.04)
            elif arguments.command == "click":
                if arguments.button < 1 or arguments.button > 8:
                    raise SystemExit("mouse button must be between 1 and 8")
                if arguments.x < 0 or arguments.y < 0:
                    raise SystemExit("mouse coordinates must be non-negative")
                client.mouseMove(arguments.x, arguments.y)
                client.mouseDown(arguments.button)
                client.pause(0.04)
                client.mouseUp(arguments.button)
                client.pause(0.08)
            elif arguments.command == "drag":
                if arguments.button < 1 or arguments.button > 8:
                    raise SystemExit("mouse button must be between 1 and 8")
                if arguments.steps < 1 or any(
                    coordinate < 0
                    for coordinate in (
                        arguments.start_x,
                        arguments.start_y,
                        arguments.end_x,
                        arguments.end_y,
                    )
                ):
                    raise SystemExit("drag coordinates must be non-negative and steps at least one")
                client.mouseMove(arguments.start_x, arguments.start_y)
                client.pause(0.04)
                client.mouseDown(arguments.button)
                client.pause(0.04)
                for step in range(1, arguments.steps + 1):
                    fraction = step / arguments.steps
                    client.mouseMove(
                        round(arguments.start_x + (arguments.end_x - arguments.start_x) * fraction),
                        round(arguments.start_y + (arguments.end_y - arguments.start_y) * fraction),
                    )
                    client.pause(0.02)
                client.mouseUp(arguments.button)
                client.pause(0.08)
            elif arguments.command == "type":
                for character in arguments.text:
                    if character == "\r":
                        continue
                    key = {"-": "minus", "\n": "enter", "\t": "tab"}.get(character, character)
                    client.keyPress(key)
                    client.pause(0.02)
            elif arguments.command == "dropdown":
                if not 0 <= arguments.index <= 256:
                    raise SystemExit("dropdown index must be between 0 and 256")
                # WayVNC keyboard lifetime follows its connection. Keep it alive
                # while GTK opens the popup and throughout the key sequence.
                client.pause(0.08)
                tool = [sys.executable, str(Path(__file__).with_name("atspi_tool.py"))]
                state = subprocess.run(
                    tool + ["inspect", arguments.query, "--exact", "--role", "combo box",
                            "--application", arguments.application],
                    capture_output=True, text=True, timeout=10,
                )
                if state.returncode:
                    raise SystemExit(state.stderr or state.stdout)
                expanded = json.loads(state.stdout)[0]["expanded"]
                for _ in range(2 if expanded else 1):
                    activation = subprocess.run(
                        tool + ["activate", arguments.query, "--exact", "--role", "toggle button",
                                "--application", arguments.application],
                        capture_output=True, text=True, timeout=10,
                    )
                    if activation.returncode:
                        raise SystemExit(activation.stderr or activation.stdout)
                    client.pause(0.08)
                client.pause(0.15)
                client.keyPress("home")
                for _ in range(arguments.index):
                    client.keyPress("down")
                client.keyPress("enter")
                client.pause(0.15)
            elif arguments.command == "key":
                key = {"escape": "esc", "return": "enter"}.get(
                    arguments.key.lower(), arguments.key.lower()
                )
                if key == "shift-tab":
                    client.keyDown("shift")
                    client.keyPress("tab")
                    client.keyUp("shift")
                else:
                    client.keyPress(key)
                client.pause(0.08)
            else:
                raise AssertionError(f"unhandled command: {arguments.command}")
    finally:
        api.shutdown()
    return 0


if __name__ == "__main__":
    sys.exit(main())
