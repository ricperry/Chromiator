#!/usr/bin/env bash
set -euo pipefail

HOST_XDG_RUNTIME_DIR="${CHROMIATOR_HOST_XDG_RUNTIME_DIR:-${XDG_RUNTIME_DIR:-/run/user/$(id -u)}}"
HOST_DBUS_SESSION_BUS_ADDRESS="${CHROMIATOR_HOST_DBUS_SESSION_BUS_ADDRESS:-${DBUS_SESSION_BUS_ADDRESS:-}}"
SKILL_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="${CHROMIATOR_REPO_ROOT:-$(git -C "$SKILL_DIR" rev-parse --show-toplevel)}"
STATE_DIR="${CHROMIATOR_WAYLAND_STATE_DIR:-$REPO_ROOT/.codex-work/gtk-wayland-debug}"
SESSION_ENV="$STATE_DIR/session.env"
APP_PID_FILE="$STATE_DIR/app.pid"
APP_UNIT_FILE="$STATE_DIR/app.unit"
ACTIVE_EVIDENCE_FILE="$STATE_DIR/active-evidence"
EVIDENCE_ROOT="${CHROMIATOR_UI_EVIDENCE_ROOT:-$REPO_ROOT/.codex-work/evidence}"

die() {
    printf 'gtk-wayland-debug: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "required command is missing: $1"
}

process_is_alive() {
    local pid="${1:-}"
    [[ "$pid" =~ ^[0-9]+$ ]] && kill -0 "$pid" 2>/dev/null
}

unit_is_active() {
    local unit="${1:-}"
    [[ -n "$unit" ]] && user_systemctl is-active --quiet "$unit" 2>/dev/null
}

user_systemctl() {
    env XDG_RUNTIME_DIR="$HOST_XDG_RUNTIME_DIR" \
        DBUS_SESSION_BUS_ADDRESS="$HOST_DBUS_SESSION_BUS_ADDRESS" \
        systemctl --user "$@"
}

user_systemd_run() {
    env XDG_RUNTIME_DIR="$HOST_XDG_RUNTIME_DIR" \
        DBUS_SESSION_BUS_ADDRESS="$HOST_DBUS_SESSION_BUS_ADDRESS" \
        systemd-run --user "$@"
}

load_session() {
    [[ -r "$SESSION_ENV" ]] || die "no private session is active; run scripts/session-start"
    # session.env contains only values written by session-start and never secrets.
    # shellcheck disable=SC1090
    source "$SESSION_ENV"
    export XDG_RUNTIME_DIR WAYLAND_DISPLAY DBUS_SESSION_BUS_ADDRESS AT_SPI_BUS_ADDRESS GTK_A11Y
    export SWAYSOCK CHROMIATOR_VNC_SERVER CHROMIATOR_SWAY_OUTPUT
    export CHROMIATOR_HOST_XDG_RUNTIME_DIR CHROMIATOR_HOST_DBUS_SESSION_BUS_ADDRESS
    unit_is_active "${CHROMIATOR_SWAY_UNIT:-}" || die "headless Sway is not running; run scripts/session-stop, then session-start"
    unit_is_active "${CHROMIATOR_WAYVNC_UNIT:-}" || die "WayVNC is not running; run scripts/session-stop, then session-start"
    [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] || die "Wayland socket is missing: $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
}

active_evidence_dir() {
    [[ -r "$ACTIVE_EVIDENCE_FILE" ]] || die "no app evidence run is active; run scripts/app-start"
    local evidence_dir
    evidence_dir="$(<"$ACTIVE_EVIDENCE_FILE")"
    [[ -d "$evidence_dir" ]] || die "active evidence directory is missing: $evidence_dir"
    printf '%s\n' "$evidence_dir"
}

resolve_vnc_python() {
    if [[ -n "${CHROMIATOR_VNC_PYTHON:-}" ]]; then
        [[ -x "$CHROMIATOR_VNC_PYTHON" ]] || die "CHROMIATOR_VNC_PYTHON is not executable: $CHROMIATOR_VNC_PYTHON"
        printf '%s\n' "$CHROMIATOR_VNC_PYTHON"
        return
    fi

    if python3 "$SKILL_DIR/scripts/vnc_client.py" check >/dev/null 2>&1; then
        command -v python3
        return
    fi

    local vncdo_path shebang_python
    vncdo_path="$(command -v vncdo 2>/dev/null || true)"
    if [[ -n "$vncdo_path" ]]; then
        shebang_python="$(sed -n '1s/^#!//p' "$vncdo_path")"
        if [[ -x "$shebang_python" ]] && "$shebang_python" "$SKILL_DIR/scripts/vnc_client.py" check >/dev/null 2>&1; then
            printf '%s\n' "$shebang_python"
            return
        fi
    fi

    die "VNCDoTool is unavailable or broken; run scripts/preflight for the dependency error"
}

run_vnc_helper() {
    load_session
    local vnc_python
    vnc_python="$(resolve_vnc_python)"
    "$vnc_python" "$SKILL_DIR/scripts/vnc_client.py" "$@"
}

stop_pid_file() {
    local pid_file="$1"
    local label="$2"
    [[ -r "$pid_file" ]] || return 0
    local pid
    pid="$(<"$pid_file")"
    if process_is_alive "$pid"; then
        kill -TERM "$pid"
        local attempt
        for attempt in {1..50}; do
            process_is_alive "$pid" || break
            sleep 0.1
        done
        process_is_alive "$pid" && die "$label did not stop after SIGTERM (pid $pid)"
    fi
    rm -f -- "$pid_file"
}

stop_app() {
    if [[ -r "$APP_UNIT_FILE" ]]; then
        local app_unit
        app_unit="$(<"$APP_UNIT_FILE")"
        if unit_is_active "$app_unit"; then
            user_systemctl stop "$app_unit"
        fi
        rm -f -- "$APP_UNIT_FILE" "$APP_PID_FILE"
        return
    fi
    stop_pid_file "$APP_PID_FILE" "Chromiator"
}
