#!/bin/sh
# Package-only launch evidence collector. Exit status is an attempt, not GUI acceptance.
set -eu

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 /absolute/path/package.AppImage /new/output/directory [fixture.png]" >&2
    exit 2
fi
package=$1
output=$2
fixture=${3:-}
case "$package" in /*) ;; *) echo "package path must be absolute" >&2; exit 2;; esac
case "$output" in /*) ;; *) echo "output path must be absolute" >&2; exit 2;; esac
[ -f "$package" ] || { echo "package does not exist: $package" >&2; exit 2; }
[ -x "$package" ] || { echo "package is not executable: $package" >&2; exit 2; }
if [ -n "$fixture" ]; then
    case "$fixture" in /*) ;; *) echo "fixture path must be absolute" >&2; exit 2;; esac
    [ -f "$fixture" ] || { echo "fixture does not exist: $fixture" >&2; exit 2; }
fi
seconds=${SMOKE_SECONDS:-90}
case "$seconds" in *[!0-9]*|'') echo "SMOKE_SECONDS must be a positive integer" >&2; exit 2;; esac
[ "$seconds" -gt 0 ] || { echo "SMOKE_SECONDS must be positive" >&2; exit 2; }
mkdir "$output" || { echo "output directory must be new: $output" >&2; exit 2; }
mkdir "$output/config" "$output/data" "$output/cache" "$output/state"
export XDG_CONFIG_HOME="$output/config"
export XDG_DATA_HOME="$output/data"
export XDG_CACHE_HOME="$output/cache"
export XDG_STATE_HOME="$output/state"

{
    printf 'package=%s\n' "$package"
    printf 'sha256='; sha256sum "$package"
    printf 'uname='; uname -a
    printf 'glibc='; getconf GNU_LIBC_VERSION || :
    printf 'time='; date -u '+%Y-%m-%dT%H:%M:%SZ'
    if [ -n "$fixture" ]; then
        printf 'fixture='; sha256sum "$fixture"
    fi
} > "$output/metadata.txt"
printf 'route\texit\tclassification\n' > "$output/attempts.tsv"

attempt() {
    route=$1
    shift
    set +e
    timeout --signal=TERM "${seconds}s" "$@" > "$output/$route.stdout" 2> "$output/$route.stderr"
    status=$?
    set -e
    classification=process_exit
    [ "$status" -eq 124 ] && classification=timed_out
    if [ "$route" = fuse ] && [ "$status" -ne 0 ] && \
       grep -Eiq 'Cannot mount AppImage|libfuse\.so|/dev/fuse|fusermount|FUSE is not available' "$output/$route.stderr"; then
        classification=fuse_or_mount_failure
    fi
    printf '%s\t%s\t%s\n' "$route" "$status" "$classification" >> "$output/attempts.tsv"
}

# No --example: this route reaches the normal welcome startup path.
attempt fuse "$package"
mkdir "$output/extract"
set +e
(cd "$output/extract" && timeout --signal=TERM "${seconds}s" "$package" --appimage-extract) \
    > "$output/extract.stdout" 2> "$output/extract.stderr"
extract_status=$?
set -e
if [ "$extract_status" -eq 0 ] && [ -x "$output/extract/squashfs-root/AppRun" ]; then
    attempt extracted "$output/extract/squashfs-root/AppRun"
else
    printf 'extracted\t%s\textraction_failed\n' "$extract_status" >> "$output/attempts.tsv"
fi
echo "Launch attempts recorded in $output; inspect UI, logs, and exported files separately."
