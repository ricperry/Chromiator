#!/usr/bin/env python3
"""Run every GTK audit scenario in an isolated process with a hard watchdog."""

from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import shutil
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests" / "artifacts" / "audit"
SCENARIOS = (
    "shell",
    "voronoi",
    "presets",
    "picker",
    "io",
    "responsive",
)
# Retained scenarios perform real widget actions and semantic readback. GTK
# process shutdown consistently adds several seconds after the completion marker.
TOTAL_TIMEOUT = 16.0
STALE_TIMEOUT = 3.0
FATAL_DIAGNOSTICS = (
    "panicked at",
    "thread caused non-unwinding panic",
    "Gtk-CRITICAL",
    "Gdk-CRITICAL",
    "GLib-CRITICAL",
    "GObject-CRITICAL",
    "GLib-GObject-CRITICAL",
    "Gtk-WARNING",
    "Gdk-WARNING",
    "GLib-WARNING",
    "GObject-WARNING",
    "GLib-GObject-WARNING",
    "Adwaita-WARNING",
    "GdkPixbuf-WARNING",
)


def parse_events(text: str) -> tuple[list[dict], int]:
    events = []
    malformed = 0
    for line in text.splitlines():
        if not line.strip():
            continue
        try:
            event = json.loads(line)
            if isinstance(event, dict):
                events.append(event)
            else:
                malformed += 1
        except json.JSONDecodeError:
            malformed += 1
    return events, malformed


def read_events(path: Path) -> tuple[list[dict], int]:
    if not path.exists():
        return [], 0
    return parse_events(path.read_text(encoding="utf-8"))


def fatal_diagnostics(text: str) -> list[str]:
    return [token for token in FATAL_DIAGNOSTICS if token in text]


def scenario_pass(
    completed: bool,
    returncode: int,
    critical: bool,
    explicit_fail: bool,
    malformed_lines: int,
) -> bool:
    return (
        completed
        and returncode == 0
        and not critical
        and not explicit_fail
        and malformed_lines == 0
    )


def main() -> int:
    # A completion marker must never hide an earlier failed assertion.
    assert not scenario_pass(True, 0, False, True, 0)
    assert not scenario_pass(True, 0, False, False, 1)
    parsed, malformed = parse_events('{"phase":"settled"}\nnot-json\n[]\n')
    assert len(parsed) == 1 and malformed == 2
    assert fatal_diagnostics("Gtk-WARNING: synthetic audit warning") == ["Gtk-WARNING"]
    OUT.mkdir(parents=True, exist_ok=True)
    binary = ROOT / "target" / "debug" / "chromiator"
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    results = []
    for scenario in SCENARIOS:
        log = OUT / f"{scenario}.jsonl"
        stderr = OUT / f"{scenario}.stderr.log"
        log.unlink(missing_ok=True)
        command = [
            str(binary),
            "--example",
            "--ui-audit-scenario",
            scenario,
            "--ui-audit-log",
            str(log),
        ]
        if scenario == "responsive":
            command += ["--window-size", "720x700"]
        env = os.environ.copy()
        env.setdefault("GSK_RENDERER", "cairo")
        xdg_data = OUT / "xdg" / scenario
        shutil.rmtree(xdg_data, ignore_errors=True)
        xdg_data.mkdir(parents=True, exist_ok=True)
        env["XDG_DATA_HOME"] = str(xdg_data)
        started = time.monotonic()
        with stderr.open("w", encoding="utf-8") as err:
            process = subprocess.Popen(command, cwd=ROOT, env=env, stderr=err)
            last_count = 0
            last_progress = started
            reason = None
            while process.poll() is None:
                now = time.monotonic()
                count = len(read_events(log)[0])
                if count != last_count:
                    last_count = count
                    last_progress = now
                if now - started > TOTAL_TIMEOUT:
                    reason = "total-timeout"
                    process.kill()
                    break
                if count and now - last_progress > STALE_TIMEOUT:
                    reason = "stale-heartbeat"
                    process.kill()
                    break
                time.sleep(0.05)
            try:
                returncode = process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                process.kill()
                returncode = process.wait()
        events, malformed_lines = read_events(log)
        last_begin = next((event for event in reversed(events) if event.get("phase") == "begin"), None)
        completed = any(
            event.get("phase") == "settled"
            and event.get("control") == "scenario"
            and event.get("action") == "complete"
            for event in events
        )
        explicit_fail = any(event.get("phase") == "fail" for event in events)
        error_text = stderr.read_text(encoding="utf-8")
        diagnostics = fatal_diagnostics(error_text)
        critical = bool(diagnostics)
        status = (
            "pass"
            if scenario_pass(completed, returncode, critical, explicit_fail, malformed_lines)
            else "fail"
        )
        results.append(
            {
                "scenario": scenario,
                "status": status,
                "reason": reason,
                "returncode": returncode,
                "events": len(events),
                "last_begun_step": last_begin,
                "gtk_or_panic_critical": critical,
                "fatal_diagnostics": diagnostics,
                "explicit_fail_event": explicit_fail,
                "malformed_jsonl_lines": malformed_lines,
                "elapsed_ms": round((time.monotonic() - started) * 1000),
            }
        )
    summary = {
        "command": "python3 scripts/run_ui_audit.py",
        "pass": sum(result["status"] == "pass" for result in results),
        "fail": sum(result["status"] == "fail" for result in results),
        "skip_events": sum(
            event.get("action") == "skip"
            for scenario in SCENARIOS
            for event in read_events(OUT / f"{scenario}.jsonl")[0]
        ),
        "results": results,
    }
    (OUT / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0 if summary["fail"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
