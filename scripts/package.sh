#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
snapshot="$(mktemp -d /tmp/carlitos-package.XXXXXXXX)"
trap 'rm -rf -- "$snapshot"' EXIT
cp Cargo.toml Cargo.lock build.rs flake.nix flake.lock LICENSE build.sh "$snapshot/"
cp -r .cargo src ui data tests scripts nix vendor "$snapshot/"
if [[ "${1:-}" == --portable ]]; then
  shift
  mkdir -p target/portable target/releases
  nix build "path:$snapshot#portable" --out-link "$PWD/target/portable/linux" "$@"
  # Evaluation does not produce a release; do not copy a stale previous build.
  for option in "$@"; do
    if [[ "$option" == --dry-run ]]; then exit 0; fi
  done
  # Nix outputs are read-only. Install writable copies so the next build can
  # replace a release, including copies created by older versions of this script.
  install -m644 target/portable/linux/*.tar.xz target/portable/linux/*.sha256 target/releases/
  echo "Linux release files: $PWD/target/releases"
else
  nix build "path:$snapshot" --out-link "$PWD/result" "$@"
fi
