#!/usr/bin/env python3
"""Capture the real Wayland window at several sizes in niri."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import argparse

parser = argparse.ArgumentParser()
parser.add_argument("--software", action="store_true")
parser.add_argument("--scale", type=float, default=1, choices=(1, 1.5, 2))
parser.add_argument("cases", nargs="*")
options = parser.parse_args()

root = Path(__file__).resolve().parents[2]
suffix = ("-software" if options.software else "") + (f"-scale-{options.scale:g}" if options.scale != 1 else "")
output = root / "target/tests" / ("visual" + suffix)
output.mkdir(parents=True, exist_ok=True)
if os.environ.get("CARLITOS_VISUAL_NESTED") != "1":
    # A separate compositor keeps tests independent of the user's workspace and lock screen.
    config = output / "niri.kdl"
    config.write_text('hotkey-overlay { skip-at-startup; }\nanimations { off; }\nprefer-no-csd\nwindow-rule { match app-id="^io.github.mny315.Carlitos$"; open-floating true; }\n')
    result = output / "result.json"
    result.unlink(missing_ok=True)
    child_env = dict(os.environ, CARLITOS_VISUAL_NESTED="1")
    if options.software:
        child_env["SLINT_BACKEND"] = "winit-software"
    with (output / "compositor.log").open("w") as compositor_log:
        compositor = subprocess.Popen(["niri", "--config", str(config), "--", sys.executable, str(Path(__file__).resolve()), *sys.argv[1:]], env=child_env, stdout=compositor_log, stderr=compositor_log)
        try:
            deadline = time.monotonic() + 240
            while time.monotonic() < deadline and compositor.poll() is None and not result.exists():
                time.sleep(.2)
            if not result.exists():
                raise RuntimeError(f"Nested compositor capture failed; see {output / 'compositor.log'}")
            report = json.loads(result.read_text())
            if isinstance(report, dict) and "error" in report:
                raise RuntimeError(report["error"])
            print(json.dumps(report, ensure_ascii=False, indent=2))
        finally:
            if compositor.poll() is None:
                compositor.terminate()
                compositor.wait(timeout=10)
    sys.exit(0)
binary = Path(os.environ.get("CARGO_TARGET_DIR", root / "target")) / "debug" / "carlitos"
def report_error(kind, value, trace):
    (output / "result.json").write_text(json.dumps({"error": str(value)}))
    sys.__excepthook__(kind, value, trace)
sys.excepthook = report_error
cases = [
    ("library-dark", "1000x700", "dark", "ru", []),
    ("library-light", "1000x700", "light", "en", []),
    ("library-narrow", "360x480", "dark", "ru", []),
    ("book-dark", "1000x700", "dark", "ru", ["--open-book", "1"]),
    ("book-narrow", "360x480", "light", "en", ["--open-book", "1"]),
    ("interaction-wide", "1000x700", "dark", "ru", ["--self-test"]),
    ("interaction-narrow", "360x480", "light", "ru", ["--self-test"]),
    ("cover-theme-dark", "1000x700", "dark", "ru", ["--test-cover-theme"]),
    ("cover-theme-light", "360x480", "light", "ru", ["--test-cover-theme"]),
    ("settings-dark", "1000x700", "dark", "ru", ["--page", "3"]),
    ("settings-narrow", "360x480", "light", "ru", ["--page", "3"]),
    ("import-narrow", "360x480", "dark", "ru", ["--page", "2"]),
    ("about", "1000x700", "light", "en", ["--page", "5"]),
    ("about-narrow", "360x480", "dark", "ru", ["--page", "5"]),
    ("about-controls", "1000x1000", "dark", "ru", ["--page", "5"]),
    ("about-controls-narrow", "360x1000", "light", "ru", ["--page", "5"]),
    ("sources", "1000x700", "dark", "ru", ["--page", "4"]),
]
if options.cases:
    unknown = set(options.cases) - {case[0] for case in cases}
    if unknown:
        raise ValueError(f"Unknown visual cases: {sorted(unknown)}")
    cases = [case for case in cases if case[0] in options.cases]
elif options.scale > 1:
    cases = [case for case in cases if case[1] == "360x480"]
subprocess.run(["niri", "msg", "output", "winit", "scale", str(options.scale)], check=True, stdout=subprocess.DEVNULL)
report = []
for name, size, theme, language, extra in cases:
    interactive = "--self-test" in extra or "--test-cover-theme" in extra
    image = output / (name + ".png")
    image.unlink(missing_ok=True)
    with (output / (name + ".log")).open("w") as log:
        p = subprocess.Popen([str(binary), "--demo", "--size", size, "--theme", theme,
                              "--language", language,
                              "--snapshot", str(image),
                              *([] if interactive else ["--quit-after", "5"]), *extra], stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline:
                if p.poll() is not None:
                    raise RuntimeError(f"{name} exited before showing its window")
                windows = json.loads(subprocess.check_output(["niri", "msg", "-j", "windows"]))
                window = next((w for w in windows if w.get("pid") == p.pid), None)
                if window:
                    wid = str(window["id"])
                    width, height = size.split("x")
                    subprocess.run(["niri", "msg", "action", "move-window-to-floating", "--id", wid], check=True, stdout=subprocess.DEVNULL)
                    subprocess.run(["niri", "msg", "action", "set-window-width", "--id", wid, width], check=True, stdout=subprocess.DEVNULL)
                    subprocess.run(["niri", "msg", "action", "set-window-height", "--id", wid, height], check=True, stdout=subprocess.DEVNULL)
                    break
                time.sleep(.05)
            else:
                raise RuntimeError(f"No window for {name}")
            # The interaction scenario quits itself after its final assertion.
            # Rendering and saving its screenshots can exceed a fixed 12 s timer.
            p.wait(timeout=45 if interactive else 15)
            if p.returncode or not image.exists():
                raise RuntimeError(f"Capture failed: {name}, see {log.name}")
            passed = "COVER THEME CHECKS PASSED" if "--test-cover-theme" in extra else "UI CHECKS PASSED"
            if interactive and passed not in Path(log.name).read_text():
                raise RuntimeError(f"Interaction checks did not finish: {name}, see {log.name}")
            print(f"{name}: {image}", flush=True)
            import struct
            with image.open("rb") as capture:
                dimensions = struct.unpack(">II", capture.read(24)[16:24])
            report.append({"name": name, "requested": size, "actual": dimensions})
            expected = tuple(round(int(value) * options.scale) for value in size.split("x"))
            if dimensions != expected:
                raise AssertionError(f"{name}: expected {expected}, captured {dimensions}")
        finally:
            if p.poll() is None:
                p.terminate()
                p.wait(timeout=5)
(output / "result.json").write_text(json.dumps(report))
