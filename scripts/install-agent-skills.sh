#!/usr/bin/env bash
# Installs the third-party agent skills used to build this project into
# .claude/skills/ (git-ignored). Sources are pinned to reviewed commits.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$ROOT/.claude/skills"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fetch() { # repo sha dir
  git -c advice.detachedHead=false clone -q --filter=blob:none "https://github.com/$1" "$WORK/$3"
  git -C "$WORK/$3" checkout -q "$2"
}

fetch leonardomso/rust-skills fd2a861ab0406a4ac536a55274d14ea6fd1ca9c9 rust
fetch dchuk/claude-code-tauri-skills 855352dfbc2527f5a7e4ed307782d2a487bffd66 tauri
fetch shadcn/ui ffd225775bbb7955a673f08bb9be4a63600d9dfb shadcn

mkdir -p "$DEST"
rm -rf "$DEST/rust-skills" "$DEST/shadcn" "$DEST"/tauri-*
mkdir -p "$DEST/rust-skills"
cp -R "$WORK/rust/SKILL.md" "$WORK/rust/rules" "$WORK/rust/LICENSE" "$DEST/rust-skills/"
cp -R "$WORK/tauri/tauri/"tauri-* "$DEST/"
cp -R "$WORK/shadcn/skills/shadcn" "$DEST/shadcn"

echo "Installed skills into $DEST:"
ls -1 "$DEST"
