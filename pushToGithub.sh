#!/usr/bin/env bash
set -euo pipefail

# Push the Git repository in the current directory to origin/main.
# Usage:
#   ./push-main.sh "Optional commit message"

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "Error: the current directory is not inside a Git repository." >&2
    exit 1
fi

if ! git remote get-url origin >/dev/null 2>&1; then
    echo "Error: this repository does not have an 'origin' remote." >&2
    exit 1
fi

if [[ -z "$(git branch --show-current)" ]]; then
    echo "Error: Git is in detached-HEAD state." >&2
    exit 1
fi

commit_message="${1:-Update $(date '+%Y-%m-%d %H:%M:%S')}"

git add --all

if git diff --cached --quiet; then
    echo "No new changes to commit."
else
    git commit -m "$commit_message"
fi

# Push the currently checked-out commit to the remote main branch.
# Git will reject the push if origin/main contains changes you do not have.
git push origin HEAD:main

