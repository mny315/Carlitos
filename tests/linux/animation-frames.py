#!/usr/bin/env python3
"""Measure actual animations on the real display; nested compositors can cap FPS."""
import json
import os
from pathlib import Path
import subprocess
import time

root = Path(__file__).resolve().parents[2]
output = root / "target/tests/animation"
output.mkdir(parents=True, exist_ok=True)
binary = Path(os.environ.get("CARGO_TARGET_DIR", root / "target")) / "debug/carlitos"
active = next(w for w in json.loads(subprocess.check_output(["niri", "msg", "-j", "workspaces"])) if w["is_focused"])
with (output / "linux.log").open("w") as log:
    app = subprocess.Popen([str(binary), "--demo", "--size", "1263x1368", "--test-animation-fps"], stdout=log, stderr=log)
    try:
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            window = next((w for w in json.loads(subprocess.check_output(["niri", "msg", "-j", "windows"])) if w.get("pid") == app.pid), None)
            if window:
                wid = str(window["id"])
                for args in (["move-window-to-workspace", "--window-id", wid, str(active["idx"])],
                             ["move-window-to-floating", "--id", wid],
                             ["move-floating-window", "--id", wid, "--x", "100", "--y", "30"],
                             ["focus-window", "--id", wid]):
                    subprocess.run(["niri", "msg", "action", *args], check=True, stdout=subprocess.DEVNULL)
                break
            time.sleep(.02)
        else:
            raise RuntimeError("Animation window did not open")
        deadline = time.monotonic() + 65
        while app.poll() is None and time.monotonic() < deadline:
            # A background workspace is deliberately throttled by Wayland.
            # Keep this temporary floating measurement window on the visible
            # workspace without stealing keyboard focus during the run.
            visible = next(w for w in json.loads(subprocess.check_output(["niri", "msg", "-j", "workspaces"])) if w["is_focused"])
            if visible["id"] != active["id"]:
                subprocess.run(["niri", "msg", "action", "move-window-to-workspace", "--window-id", wid,
                                "--focus", "false", str(visible["idx"])], check=True, stdout=subprocess.DEVNULL)
                active = visible
            time.sleep(.1)
        assert app.wait(timeout=1) == 0
    finally:
        if app.poll() is None:
            app.terminate()
            app.wait(timeout=10)
report = next(line.removeprefix("ANIMATION FRAMES ") for line in (output / "linux.log").read_text().splitlines() if line.startswith("ANIMATION FRAMES "))
(output / "linux.json").write_text(json.dumps(json.loads(report), indent=2))
print(report)
