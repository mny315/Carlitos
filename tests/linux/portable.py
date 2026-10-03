#!/usr/bin/env python3
"""Test the extracted release in a mount namespace without /nix or host codecs."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--media-test", required=True, type=Path)
    parser.add_argument("--fixtures", required=True, type=Path)
    parser.add_argument("--busybox", default=os.environ.get("CARLITOS_TEST_BUSYBOX"))
    parser.add_argument("--gui", action="store_true")
    parser.add_argument("--real-audio", action="store_true", help="also test muted output through the session's PulseAudio/PipeWire socket")
    options = parser.parse_args()
    if not options.busybox:
        parser.error("--busybox must name a statically linked BusyBox")
    project = Path(__file__).resolve().parents[2]
    output = project / "target/tests/portable"
    output.mkdir(parents=True, exist_ok=True)
    (output / "result.json").unlink(missing_ok=True)
    (output / "window.png").unlink(missing_ok=True)
    archive = options.bundle.parent / (options.bundle.name + ".tar.xz")
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    expected = Path(str(archive) + ".sha256").read_text().split()[0]
    if digest != expected:
        raise RuntimeError("Release archive checksum mismatch")
    with tempfile.TemporaryDirectory(prefix="carlitos-portable-") as temporary:
        # A moved directory with spaces catches hard-coded build/extraction paths.
        moved = Path(temporary) / "Переносимая книга with spaces"
        with tarfile.open(archive, "r:xz") as tar:
            tar.extractall(temporary, filter="data")
        (Path(temporary) / options.bundle.name).rename(moved)
        manifest = json.loads((moved / "runtime-manifest.json").read_text())
        for relative, entry in manifest["files"].items():
            if hashlib.sha256((moved / relative).read_bytes()).hexdigest() != entry["sha256"]:
                raise RuntimeError(f"Bundled file checksum mismatch: {relative}")
        for directory in [moved, moved / "libexec"]:
            directory.chmod(0o755)
        test = moved / "libexec/media-test"
        shutil.copyfile(options.media_test, test)
        test.chmod(0o755)
        subprocess.run(["patchelf", "--set-rpath", "$ORIGIN/../lib", str(test)], check=True)
        interpreter = subprocess.check_output(
            ["patchelf", "--print-interpreter", str(options.media_test)], text=True).strip()
        loader = Path(interpreter).name
        command = [
            "bwrap", "--die-with-parent", "--unshare-pid", "--unshare-net",
            "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp",
            "--dir", "/bin", "--dir", "/etc", "--dir", "/nix",
            "--dir", "/home/test", "--dir", "/run",
            "--ro-bind", str(Path(options.busybox).resolve()), "/bin/sh",
            "--ro-bind", str(Path(options.busybox).resolve()), "/bin/dirname",
            "--ro-bind", str(moved), "/app with spaces",
            "--ro-bind", str(options.fixtures.resolve()), "/fixtures",
            "--bind", str(output), "/output",
            "--clearenv", "--setenv", "PATH", "/bin",
            "--setenv", "HOME", "/home/test", "--setenv", "LANG", "C.UTF-8",
            "--setenv", "GST_PLUGIN_SYSTEM_PATH_1_0", "/nonexistent-host-plugins",
            "--setenv", "GST_PLUGIN_PATH_1_0", "/nonexistent-host-plugins",
            "--chdir", "/app with spaces",
        ]
        def run(arguments, label, extra=(), timeout=90):
            result = subprocess.run([*command, *extra, *arguments], text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                    timeout=timeout)
            (output / f"{label}.log").write_text(result.stdout)
            if result.returncode:
                raise RuntimeError(f"{label} failed ({result.returncode}):\n{result.stdout}")
            print(f"PASS: {label}", flush=True)
            return result.stdout
        run(["./Carlitos", "--help"], "launch-without-nix")
        # Use precisely the release's libraries/plugins with our real Rust tests.
        audio = []
        if options.real_audio:
            runtime = os.environ["XDG_RUNTIME_DIR"]
            audio = ["--bind", runtime, runtime, "--setenv", "XDG_RUNTIME_DIR", runtime,
                     "--setenv", "PULSE_SERVER", f"unix:{runtime}/pulse/native",
                     "--setenv", "CARLITOS_REAL_AUDIO", "1"]
        run(["/bin/sh", "-c", f'''
            export GST_PLUGIN_SYSTEM_PATH_1_0='/app with spaces/lib/gstreamer-1.0'
            export GST_PLUGIN_PATH_1_0=
            export GST_PLUGIN_SCANNER_1_0='/app with spaces/libexec/gst-plugin-scanner'
            export GST_REGISTRY_1_0=/tmp/test-registry.bin
            export CARLITOS_FIXTURES=/fixtures
            exec './lib/{loader}' --inhibit-cache --library-path './lib' \
                ./libexec/media-test --ignored --nocapture --test-threads=1
        '''], "codecs-seek-speed-import", audio)
        if options.gui:
            runtime = os.environ["XDG_RUNTIME_DIR"]
            wayland = os.environ["WAYLAND_DISPLAY"]
            extra = ["--bind", runtime, runtime, "--setenv", "XDG_RUNTIME_DIR", runtime,
                     "--setenv", "WAYLAND_DISPLAY", wayland]
            run(["./Carlitos", "--demo", "--no-desktop", "--snapshot", "/output/window.png",
                 "--quit-after", "5"], "window-without-nix", extra)
            if not (output / "window.png").is_file():
                raise RuntimeError("No screenshot produced by the portable release")
        report = {"passed": True, "bundle": str(options.bundle.resolve()),
                  "archive_sha256": digest,
                  "without_nix": True, "without_host_codecs": True,
                  "relocated": True, "gui": options.gui, "real_audio": options.real_audio}
        (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
