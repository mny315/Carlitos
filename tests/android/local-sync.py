#!/usr/bin/env python3
"""Real Wi-Fi progress roundtrip with QR decoding in the isolated Android APK."""
import argparse
import json
import os
import signal
from pathlib import Path
import subprocess
import time

parser = argparse.ArgumentParser()
parser.add_argument("--adb", required=True)
parser.add_argument("--app-apk", type=Path, help="Reuse a built isolated APK for camera/UI test changes")
parser.add_argument("--keep-awake", action="store_true", help="Leave the phone unlocked after the test")
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
output = root / "target/tests/android/local-sync"
output.mkdir(parents=True, exist_ok=True)
adb = args.adb
package = "io.github.mny315.carlitos.playbacktest"
apk = args.app_apk or root / "android/app/build/outputs/apk/debug/app-universal-debug.apk"
test_apk = root / "android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk"
versions = json.loads((root / "nix/android-versions.json").read_text())
aapt = Path(adb).absolute().parents[1] / "build-tools" / versions["buildTools"] / "aapt2"
badging = subprocess.check_output([str(aapt), "dump", "badging", str(apk)], text=True)
assert f"package: name='{package}'" in badging, "Build the isolated playbacktest APK first"
subprocess.run([adb, "install", "-r", str(apk)], check=True)
subprocess.run([adb, "install", "-r", str(test_apk)], check=True)
subprocess.run([adb, "shell", "am", "force-stop", package], check=True)
subprocess.run([adb, "shell", "pm", "clear", package], check=True)
ready = output / "ready"
ready.unlink(missing_ok=True)
env = dict(os.environ, CARLITOS_SYNC_DEVICE_DIR=str(output), CARGO_TARGET_DIR=str(root / "target"))
with (output / "desktop.log").open("w") as log:
    host = subprocess.Popen(["./build.sh", "test", "unit", "--lib", "android_qr_and_progress_roundtrip", "--", "--ignored", "--nocapture"], cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    try:
        deadline = time.monotonic() + 120
        while not ready.exists():
            if host.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError(f"Desktop peer did not start; see {log.name}")
            time.sleep(.1)
        subprocess.run([adb, "shell", "run-as", package, "mkdir", "-p", "files/sync"], check=True)
        for name in ("book.wav", "pair.png"):
            subprocess.run([adb, "shell", "-T", f"run-as {package} sh -c 'cat > files/sync/{name}'"], input=(output / name).read_bytes(), check=True)
        result = subprocess.run([adb, "shell", "am", "instrument", "-e", "sync-only", "true", "-w", "io.github.mny315.carlitos.test/io.github.mny315.carlitos.DeviceInstrumentation"], text=True, capture_output=True, timeout=90)
        (output / "android.log").write_text(result.stdout + result.stderr)
        subprocess.run([adb, "pull", f"/sdcard/Android/data/{package}/files/ui", str(output)], check=False, stdout=subprocess.DEVNULL)
        print(result.stdout, end="", flush=True)
        assert result.returncode == 0 and "PASS Android local sync and QR decoding" in result.stdout, "Android sync failed"
        assert host.wait(timeout=20) == 0, f"Desktop verification failed; see {log.name}"
        print("PASS real Android ↔ Linux local-network progress, rewind, reset and QR decoding")
    finally:
        if host.poll() is None:
            os.killpg(host.pid, signal.SIGTERM)
            host.wait(timeout=20)
        subprocess.run([adb, "shell", "am", "force-stop", package], check=False)
        if not args.keep_awake:
            subprocess.run([adb, "shell", "input", "keyevent", "KEYCODE_SLEEP"], check=False)
