#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
task="${1:-menu}"
if [[ $# -gt 0 ]]; then shift; fi
# Keep a fresh log for each action/group, including failures in the Nix shell.
if [[ "$task" != menu && "$task" != help && "$task" != -h && "$task" != --help && "${CARLITOS_BUILD_LOG_ACTIVE:-}" != 1 ]]; then
  log_name="$task"
  if [[ "$task" == test || "$task" == check || "$task" == android ]]; then
    log_name+="-${1:-default}"
    if [[ "$task" == test && "${1:-}" == android ]]; then log_name+="-${2:-session}"; fi
  fi
  log_name="${log_name//[^a-zA-Z0-9_-]/_}"
  mkdir -p target/logs
  log_file="$PWD/target/logs/$log_name.log"
  set +e
  CARLITOS_BUILD_LOG_ACTIVE=1 ./build.sh "$task" "$@" 2>&1 | tee "$log_file"
  statuses=("${PIPESTATUS[@]}")
  printf '\nLog: %s\n' "$log_file"
  if [[ "${statuses[0]}" -ne 0 ]]; then exit "${statuses[0]}"; fi
  exit "${statuses[1]}"
fi
usage() {
  cat <<'HELP'
Usage: ./build.sh <command> [options]
  menu                    Interactive build and test menu (default)
  run | demo              Run the Linux app
  build                   Native release binary (Cargo options accepted)
  linux                   Portable Linux archive (Nix options accepted)
  windows                 Windows x64 installer (Cargo options accepted)
  android [release|debug|install|run|devices|keygen]
                          Signed release APK by default; install/run use debug
  all                     Linux, Windows and signed Android releases
  check [linux|android|all] Formatting and Clippy (Linux by default)
  test [unit|tooling|media|sonic|ui|desktop|visual|performance|portable|windows]
                          Unit + integration tests by default; options forwarded
  test android [session|sources|import|playback|ui]
                          Device tests; playback/UI use an isolated test app
  format | fetch | versions
  package | install       Build/install the native Nix package
  sources                 Export sources for the portable Linux release

Release files: target/releases/    Test reports: target/tests/
Logs (overwritten on each run): target/logs/<command>-<group>.log
HELP
}
case "$task" in
  help|-h|--help) usage; exit 0 ;;
  menu) exec bash scripts/menu.sh "$@" ;;
  android) exec bash scripts/android.sh "$@" ;;
  linux) exec bash scripts/package.sh --portable "$@" ;;
  all)
    if [[ $# -ne 0 ]]; then usage >&2; exit 2; fi
    ./build.sh linux
    ./build.sh windows
    exec ./build.sh android release
    ;;
  check)
    platform="${1:-linux}"
    if [[ $# -gt 0 ]]; then shift; fi
    if [[ $# -ne 0 ]]; then usage >&2; exit 2; fi
    case "$platform" in
      linux) ;;
      android) exec bash scripts/android.sh check ;;
      all) ./build.sh check linux; exec ./build.sh check android ;;
      *) usage >&2; exit 2 ;;
    esac
    ;;
  test)
    group="${1:-unit}"
    if [[ $# -gt 0 ]]; then shift; fi
    case "$group" in
      android) exec bash scripts/android.sh test "${@}" ;;
      tooling) exec python3 tests/tooling/cli.py "$@" ;;
      unit|media|sonic|ui|desktop|visual|performance|portable|windows) ;;
      *) usage >&2; exit 2 ;;
    esac
    ;;
  format|build|windows|versions|fetch|run|demo|package|sources|install) ;;
  *) usage >&2; exit 2 ;;
esac

dev_shell=default
dev_marker=1
if [[ "$task" == windows || ( "$task" == test && "$group" == windows ) ]]; then
  dev_shell=windows
  dev_marker=windows
fi
if [[ "${CARLITOS_DEV_SHELL:-}" != "$dev_marker" ]]; then
  # Snapshot only toolchain definitions, never build outputs or signing keys.
  snapshot="$(mktemp -d /tmp/carlitos-nix.XXXXXXXX)"
  trap 'rm -rf -- "$snapshot"' EXIT
  cp -- flake.nix flake.lock "$snapshot/"
  cp -r -- nix "$snapshot/"
  args=("$task")
  if [[ "$task" == test ]]; then args+=("$group"); fi
  nix develop "path:$snapshot#$dev_shell" --command env CARLITOS_DEV_SHELL="$dev_marker" ./build.sh "${args[@]}" "$@"
  exit
fi
case "$task" in
  format) cargo fmt "$@" ;;
  check) cargo fmt --check; cargo clippy --locked --all-targets -- -D warnings ;;
  test)
    case "$group" in
      unit) cargo test --locked "$@" ;;
      media) bash tests/audio/media.sh "$@" ;;
      sonic) bash tests/audio/sonic.sh "$@" ;;
      ui) cargo run --locked -- --self-test "$@" ;;
      desktop|visual|performance) cargo build --locked; python3 "tests/linux/$group.py" "$@" ;;
      portable) bash tests/linux/portable.sh "$@" ;;
      windows) xvfb-run -a python3 tests/windows/installer.py "$@" ;;
    esac
    ;;
  build) cargo build --release --locked "$@" ;;
  windows) bash scripts/windows.sh "$@" ;;
  fetch) cargo fetch --locked "$@" ;;
  run) cargo run --locked -- "$@" ;;
  demo) cargo run --locked -- --demo "$@" ;;
  package) bash scripts/package.sh "$@" ;;
  sources) python3 scripts/portable-sources.py target/portable/linux/Carlitos-*-linux "$@" ;;
  install) bash scripts/install.sh "$@" ;;
  versions) rustc --version; cargo --version; pkg-config --modversion gstreamer-1.0 ;;
esac
