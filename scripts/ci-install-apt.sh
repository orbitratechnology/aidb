#!/usr/bin/env bash
# Installs system packages in CI. Downloaded .deb files are kept in a directory that
# actions/cache saves between runs, so a slow Ubuntu mirror only has to serve package lists and
# packages that changed. Stalled downloads are retried.
#
# Usage: scripts/ci-install-apt.sh <package>...
set -euo pipefail

archives="${APT_ARCHIVES:-$HOME/.cache/apt-archives}"
mkdir -p "$archives/partial"

printf 'Acquire::Retries "3";\nAcquire::http::Timeout "60";\n' |
  sudo tee /etc/apt/apt.conf.d/80-ci-retries >/dev/null
sudo apt-get update
sudo apt-get install -y --no-install-recommends -o Dir::Cache::Archives="$archives" "$@"
# Drop package versions the mirror no longer offers, so the cache does not grow forever.
sudo apt-get autoclean -o Dir::Cache::Archives="$archives"
sudo chown -R "$(id -u):$(id -g)" "$archives"
