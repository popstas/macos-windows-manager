#!/usr/bin/env bash
# Rebuilds CHANGELOG.md from commits. With no argument it rewrites the whole
# file; with a tag it prints that release's notes to stdout — a draft for
# `gh release`, leaving the file alone.
#
#   ./scripts/changelog.sh            # rebuild CHANGELOG.md
#   ./scripts/changelog.sh unreleased # what piled up after the last tag
#   ./scripts/changelog.sh v0.7.0     # notes for v0.7.0, to stdout
set -euo pipefail

cd "$(dirname "$0")/.."

command -v git-cliff >/dev/null || {
  echo "git-cliff not found: cargo install git-cliff (or pip install git-cliff)" >&2
  exit 1
}

case "${1:-}" in
  "")
    git-cliff -o CHANGELOG.md
    echo "CHANGELOG.md rebuilt" >&2
    ;;
  unreleased)
    git-cliff --unreleased --strip header
    ;;
  *)
    tag="$1"
    git tag --list "$tag" | grep -qx "$tag" || { echo "no such tag: $tag" >&2; exit 1; }
    # The section is cut out of the whole changelog rather than built from a
    # `prev..$tag` range: the very first tag has no predecessor, so no range
    # can be written for it — and `--latest` always means the newest tag,
    # not the one that was asked for.
    git-cliff --strip header 2>/dev/null |
      awk -v tag="## $tag " 'index($0, tag) == 1 { on = 1; print; next }
                             on && /^## / { exit }
                             on { print }'
    ;;
esac
