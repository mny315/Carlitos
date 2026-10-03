#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
task="${1:-release}"
if [[ $# -gt 0 ]]; then shift; fi
suite=
profile=debug
case "$task" in
  test)
    suite="${1:-session}"
    if [[ $# -gt 0 ]]; then shift; fi
    case "$suite" in
      session) ;;
      sources|import|playback|ui) task="test-$suite" ;;
      *) echo "Unknown Android test group: $suite" >&2; exit 2 ;;
    esac
    ;;
  native)
    profile="${1:-debug}"
    if [[ $# -gt 0 ]]; then shift; fi
    if [[ "$profile" != debug && "$profile" != release ]]; then exit 2; fi
    ;;
  release|debug|check|install|run|devices|keygen) ;;
  *) echo "Usage: ./build.sh android [release|debug|check|install|run|devices|keygen]" >&2; exit 2 ;;
esac
if [[ $# -gt 0 ]]; then echo "Unexpected arguments: $*" >&2; exit 2; fi

# Nix supplies a separate Android environment. Other systems can supply the
# pinned SDK/NDK, JDK 17, Rustup and Python and set CARLITOS_ANDROID_SHELL=1.
if [[ "${CARLITOS_ANDROID_SHELL:-}" != 1 ]]; then
  snapshot="$(mktemp -d /tmp/carlitos-android-nix.XXXXXXXX)"
  trap 'rm -rf -- "$snapshot"' EXIT
  cp flake.nix flake.lock "$snapshot/"
  cp -r nix "$snapshot/"
  args=("$task")
  if [[ -n "$suite" ]]; then args=(test "$suite"); fi
  if [[ "$task" == native ]]; then args+=("$profile"); fi
  nix develop "path:$snapshot#android" --command env CARLITOS_ANDROID_SHELL=1 bash scripts/android.sh "${args[@]}"
  exit
fi

if [[ "$task" == keygen ]]; then exec python3 scripts/android-key.py; fi
if [[ "$task" == release ]]; then
  if [[ ! -f .secrets/android/signing.properties ]]; then
    echo 'Create the release key first: ./build.sh android keygen' >&2
    exit 1
  fi
  unset CARLITOS_ANDROID_IMPORT_TESTS CARLITOS_ANDROID_PLAYBACK_TESTS CARLITOS_ANDROID_UI_TESTS
fi

version() {
  python3 -c 'import json, sys; print(json.load(open("nix/android-versions.json"))[sys.argv[1]])' "$1"
}
export ANDROID_HOME="${ANDROID_HOME:?Set ANDROID_HOME to the pinned Android SDK}"
export ANDROID_NDK_ROOT="${ANDROID_NDK_ROOT:-$ANDROID_HOME/ndk/$(version ndk)}"
export ANDROID_NDK_HOME="$ANDROID_NDK_ROOT"
export ANDROID_NDK="$ANDROID_NDK_ROOT"
export GRADLE_USER_HOME="${GRADLE_USER_HOME:-$PWD/target/android-tools/gradle}"
export RUSTUP_HOME="${RUSTUP_HOME:-$PWD/target/android-tools/rustup}"
export CARGO_TARGET_DIR="$PWD/target/android/rust"
adb="$ANDROID_HOME/platform-tools/adb"
variant=debug
if [[ "$task" == release ]]; then variant=release; fi
apk_dir="$PWD/android/app/build/outputs/apk/$variant"
apk="$apk_dir/app-universal-$variant.apk"

case "$task" in
  test|test-sources|test-import|test-playback|test-ui)
    # Device suites may wake the screen for input. Always leave it asleep,
    # including when a build, assertion or instrumentation process fails.
    trap '"$adb" shell input keyevent KEYCODE_SLEEP >/dev/null 2>&1 || true' EXIT
    ;;
esac

if [[ "$task" == devices ]]; then exec "$adb" devices -l; fi
if [[ "$task" == native || "$task" == check ]]; then
  rust="$(version rust)"
  targets=(aarch64-linux-android armv7-linux-androideabi)
  if ! rustup run "$rust" rustc --version >/dev/null 2>&1; then
    rustup toolchain install "$rust" --profile minimal --component clippy --no-self-update
  else
    if [[ "$task" == check ]]; then rustup component add --toolchain "$rust" clippy; fi
  fi
  rustup target add --toolchain "$rust" "${targets[@]}"
  host=linux-x86_64
  if [[ "$(uname -s)" == Darwin ]]; then host=darwin-x86_64; fi
  compiler="$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/$host/bin"
  export CC_aarch64_linux_android="$compiler/aarch64-linux-android$(version minSdk)-clang"
  export CXX_aarch64_linux_android="$compiler/aarch64-linux-android$(version minSdk)-clang++"
  export AR_aarch64_linux_android="$compiler/llvm-ar"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
  export CC_armv7_linux_androideabi="$compiler/armv7a-linux-androideabi$(version minSdk)-clang"
  export CXX_armv7_linux_androideabi="$compiler/armv7a-linux-androideabi$(version minSdk)-clang++"
  export AR_armv7_linux_androideabi="$compiler/llvm-ar"
  export CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER="$CC_armv7_linux_androideabi"
  cargo_profile=android
  if [[ "$profile" == release ]]; then
    cargo_profile=release
    unset CARLITOS_ANDROID_IMPORT_TESTS CARLITOS_ANDROID_PLAYBACK_TESTS CARLITOS_ANDROID_UI_TESTS
  fi
  native_options=()
  if [[ "${CARLITOS_ANDROID_IMPORT_TESTS:-}" == 1 ]]; then native_options+=(--features android-import-tests); fi
  if [[ "${CARLITOS_ANDROID_PLAYBACK_TESTS:-}" == 1 ]]; then native_options+=(--features android-playback-tests); fi
  if [[ "${CARLITOS_ANDROID_UI_TESTS:-}" == 1 ]]; then native_options+=(--features android-ui-tests); fi
  for target in "${targets[@]}"; do
    if [[ "$task" == check ]]; then
      rustup run "$rust" cargo clippy --locked --lib --profile "$cargo_profile" --target "$target" "${native_options[@]}" -- -D warnings
      continue
    fi
    case "$target" in
      aarch64-linux-android) abi=arm64-v8a ;;
      armv7-linux-androideabi) abi=armeabi-v7a ;;
    esac
    # Override only this build: desktop keeps its rlib and avoids an extra DLL
    # with a colliding Windows PDB filename.
    rustup run "$rust" cargo rustc --locked --lib --profile "$cargo_profile" --crate-type cdylib --target "$target" "${native_options[@]}"
    mkdir -p "android/app/build/rust/$profile/jniLibs/$abi"
    install -m644 "$CARGO_TARGET_DIR/$target/$cargo_profile/libcarlitos.so" "android/app/build/rust/$profile/jniLibs/$abi/"
  done
  exit
fi

gradle_options=(--no-daemon --console=plain)
if [[ "$task" == test-ui ]]; then export CARLITOS_ANDROID_UI_TESTS=1; fi
if [[ "$task" == test-playback || "$task" == test-ui ]]; then
  export CARLITOS_ANDROID_PLAYBACK_TESTS=1
  python3 tests/android/playback-fixtures.py
fi
if [[ "$task" == test-import ]]; then
  export CARLITOS_ANDROID_IMPORT_TESTS=1
  CARGO_TARGET_DIR="$PWD/target" CARLITOS_ANDROID_FIXTURES="$PWD/target/tests/android/import/fixtures/Carlitos-import" \
    ./build.sh test unit --lib prepare_android_import_fixtures -- --ignored
fi
if [[ -n "${CARLITOS_AAPT2:-}" ]]; then
  gradle_options+=("-Pandroid.aapt2FromMavenOverride=$CARLITOS_AAPT2")
fi
gradle_tasks=(:app:assembleDebug)
if [[ "$variant" == release ]]; then gradle_tasks=(:app:assembleRelease); fi
if [[ "$task" == test || "$task" == test-sources || "$task" == test-import || "$task" == test-playback || "$task" == test-ui ]]; then
  gradle_tasks+=(:app:assembleDebugAndroidTest :fixtures:assembleDebug)
fi
android/gradlew -p android "${gradle_options[@]}" "${gradle_tasks[@]}"
echo "Android APK: $apk"
if [[ "$task" == release || "$task" == debug || "$task" == install || "$task" == run ]]; then
  mkdir -p target/releases
  app_version="$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["package"]["version"])')"
  for abi in armeabi-v7a arm64-v8a universal; do
    source_apk="$apk_dir/app-$abi-$variant.apk"
    name="Carlitos-$app_version-android-$abi-$variant.apk"
    if [[ "$variant" == release ]]; then
      "$ANDROID_HOME/build-tools/$(version buildTools)/apksigner" verify --verbose --print-certs "$source_apk"
    fi
    install -m644 "$source_apk" "target/releases/$name"
    (cd target/releases; sha256sum "$name" > "$name.sha256")
    echo "Standalone APK: $PWD/target/releases/$name"
  done
fi
if [[ "$task" == install || "$task" == run ]]; then "$adb" install -r "$apk"; fi
if [[ "$task" == run ]]; then
  "$adb" shell am start -n io.github.mny315.carlitos/.MainActivity
fi

if [[ "$task" == test || "$task" == test-sources || "$task" == test-import || "$task" == test-playback || "$task" == test-ui ]]; then
  "$adb" install -r "$apk"
  "$adb" install -r android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
  "$adb" install -r android/fixtures/build/outputs/apk/debug/fixtures-debug.apk
  if [[ "$task" == test-playback || "$task" == test-ui ]]; then
    # These suites edit the same synthetic books. Start each run with the
    # isolated package's fresh database, even after another suite failed.
    "$adb" shell pm clear io.github.mny315.carlitos.playbacktest
  fi
  instrument_args=()
  expected="PASS Android session tests"
  if [[ "$task" == test-sources ]]; then instrument_args=(-e suite sources); expected="PASS Android source tests"; fi
  if [[ "$task" == test-import ]]; then instrument_args=(-e suite import); expected="PASS Android import tests"; fi
  if [[ "$task" == test-playback ]]; then instrument_args=(-e suite playback); expected="PASS Android playback tests"; fi
  if [[ "$task" == test-ui ]]; then instrument_args=(-e suite ui); expected="PASS Android UI tests"; fi
  mkdir -p target/tests/android
  result_file="$PWD/target/tests/android/$task.log"
  "$adb" shell am instrument "${instrument_args[@]}" -w io.github.mny315.carlitos.test/io.github.mny315.carlitos.DeviceInstrumentation | tee "$result_file"
  result="$(cat "$result_file")"
  if [[ "$result" != *"$expected"* || "$result" == *"FAIL "* ]]; then exit 1; fi
  if [[ "$task" == test-playback ]]; then
    python3 tests/android/playback-lifecycle.py --adb "$adb"
  fi
fi
