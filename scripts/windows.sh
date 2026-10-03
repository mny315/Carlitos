#!/usr/bin/env bash
# Linux -> Windows MSVC installer. Native Windows tests/build: windows.ps1.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
if [[ "${CARLITOS_DEV_SHELL:-}" != windows ]]; then
  exec ./build.sh windows "$@"
fi

target=x86_64-pc-windows-msvc
toolchain="${CARLITOS_WINDOWS_TOOLCHAIN:-$CARLITOS_RUST_VERSION}"
export RUSTUP_HOME="${RUSTUP_HOME:-$PWD/target/windows-tools/rustup}"
export XWIN_CACHE_DIR="${XWIN_CACHE_DIR:-$PWD/target/windows-tools/xwin}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target/windows}"
export XWIN_ARCH=x86_64
mkdir -p target/windows-tools
echo "Preparing Windows x64 toolchain (Rust $toolchain)..."

# Keep the cross toolchain local to the project and match the pinned Nix Rust
# version. Subsequent builds reuse the compiler and downloaded Windows SDK.
if ! rustup run "$toolchain" rustc --version >/dev/null 2>&1; then
  rustup toolchain install "$toolchain" --profile minimal --target "$target" --no-self-update
else
  rustup target add --toolchain "$toolchain" "$target"
fi

report="$(mktemp "$PWD/target/windows-tools/build.XXXXXXXX.json")"
trap 'rm -f -- "$report"' EXIT
echo "Building Windows x64 executable..."
rustup run "$toolchain" cargo xwin build --locked --release --target "$target" "$@" \
  --message-format=json-render-diagnostics > "$report"

# Cargo reports the actual artifact, including a custom --target-dir.
# Never package a stale EXE left by an earlier build.
build_info="$(python3 - "$report" <<'PY'
import json
import sys
import tomllib

executable = None
package_id = None
assets = {}
with open(sys.argv[1]) as report:
    for line in report:
        # cargo-xwin may write its own status/log lines alongside Cargo JSON.
        if not line.lstrip().startswith("{"):
            continue
        event = json.loads(line)
        if (event.get("reason") == "compiler-artifact"
                and event.get("target", {}).get("name") == "carlitos"
                and "bin" in event["target"]["kind"]
                and not event.get("profile", {}).get("test", False)
                and event.get("executable")):
            executable = event["executable"]
            package_id = event["package_id"]
        elif event.get("reason") == "build-script-executed":
            assets[event["package_id"]] = event["out_dir"]
if executable is None:
    sys.exit("Windows build did not produce the Carlitos executable")
if package_id not in assets:
    sys.exit("Cargo did not report the installer assets directory")
print(executable)
print(assets[package_id])
with open("Cargo.toml", "rb") as manifest:
    print(tomllib.load(manifest)["package"]["version"])
PY
)"
mapfile -t build_info <<< "$build_info"
executable="${build_info[0]}"
python3 tests/windows/package.py "$executable"
# A private virtual display keeps Wine setup and compilation non-interactive,
# including on machines without a desktop session.
xvfb-run -a bash scripts/windows-installer.sh "${build_info[@]}"
