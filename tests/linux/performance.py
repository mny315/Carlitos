#!/usr/bin/env python3
"""Release measurements with real audio, isolated data/bus and a nested compositor."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import wave

root = Path(__file__).resolve().parents[2]
out = root / "target/tests/performance"
out.mkdir(parents=True, exist_ok=True)
report = out / "result.json"
if os.environ.get("CARLITOS_PERF_NESTED") != "1":
    report.unlink(missing_ok=True)
    config = out / "niri.kdl"
    config.write_text('hotkey-overlay { skip-at-startup; }\nanimations { off; }\nwindow-rule { open-floating true; }\n')
    with (out / "compositor.log").open("w") as log:
        compositor = subprocess.Popen(["niri", "--config", str(config), "--", sys.executable, str(Path(__file__).resolve())], env=dict(os.environ, CARLITOS_PERF_NESTED="1"), stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 600
            while not report.exists() and compositor.poll() is None and time.monotonic() < deadline:
                time.sleep(.2)
            result = json.loads(report.read_text())
            print(json.dumps(result, indent=2))
            if "error" in result:
                raise RuntimeError(result["error"])
        finally:
            compositor.terminate()
            compositor.wait(timeout=10)
    sys.exit(0)

def fail(kind, value, traceback):
    report.write_text(json.dumps({"error": str(value)}))
    sys.__excepthook__(kind, value, traceback)
sys.excepthook = fail
debug = Path(os.environ.get("CARGO_TARGET_DIR", root / "target")) / "debug/carlitos"
release = root / "result/bin/Carlitos"
assert release.exists(), "Build ./build.sh package first"
processes = []
config = out / "bus.conf"
config.write_text('<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>')
bus = subprocess.check_output(["dbus-daemon", f"--config-file={config}", "--fork", "--print-address=1", "--print-pid=1"], text=True).splitlines()
address, bus_pid = bus[0], int(bus[1])
environment = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=address)
# The package must supply its runtime libraries/plugins itself.
environment.pop("LD_LIBRARY_PATH", None)
environment.pop("GST_PLUGIN_SYSTEM_PATH_1_0", None)
name, path = "org.mpris.MediaPlayer2.Carlitos", "/org/mpris/MediaPlayer2"
player = "org.mpris.MediaPlayer2.Player"

def prop(key):
    return json.loads(subprocess.check_output(["busctl", f"--address={address}", "--json=short", "get-property", name, path, player, key], stderr=subprocess.DEVNULL, timeout=5))["data"]
def call(method, interface=player):
    subprocess.run(["busctl", f"--address={address}", "call", name, path, interface, method], check=True, stdout=subprocess.DEVNULL, timeout=5)
def until(predicate, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            if predicate(): return
        except subprocess.CalledProcessError: pass
        time.sleep(.02)
    raise RuntimeError("Timed out waiting for release app")
def window():
    return next((w for w in json.loads(subprocess.check_output(["niri", "msg", "-j", "windows"])) if w.get("pid") == app.pid), None)
def sample():
    fields = Path(f"/proc/{app.pid}/stat").read_text().split(")", 1)[1].split()
    cpu = (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")
    rss = int(Path(f"/proc/{app.pid}/statm").read_text().split()[1]) * os.sysconf("SC_PAGE_SIZE") / 1048576
    return cpu, rss
def measure():
    before = sample()
    started = time.monotonic()
    time.sleep(45)
    after = sample()
    return {"seconds": round(time.monotonic() - started, 2), "cpu_percent_one_core": round((after[0] - before[0]) / (time.monotonic() - started) * 100, 2), "rss_mib": round(after[1], 2), "rss_change_mib": round(after[1] - before[1], 2)}

try:
    with tempfile.TemporaryDirectory(prefix="carlitos-perf-") as temp, (out / "app.log").open("w") as log:
        temp = Path(temp)
        folder = temp / "book"
        folder.mkdir()
        for index in range(3):
            with wave.open(str(folder / f"{index}.wav"), "wb") as audio:
                audio.setnchannels(1); audio.setsampwidth(2); audio.setframerate(8000)
                audio.writeframes(bytes(8000 * 2 * 180))
        args = ["--data-dir", str(temp / "data")]
        # Seed through the actual import UI, then measure only the packaged executable.
        seed = subprocess.Popen([str(debug), *args, "--exercise", str(folder)], env=dict(os.environ, DBUS_SESSION_BUS_ADDRESS=address), stdout=log, stderr=log)
        processes.append(seed)
        until(lambda: "UI SCENARIO READY" in (out / "app.log").read_text(), 50)
        call("Quit", "org.mpris.MediaPlayer2")
        seed.wait(timeout=10)
        host = subprocess.Popen([str(debug), "--mock-tray-host"], env=environment, stdout=log, stderr=log)
        processes.append(host)
        started = time.monotonic()
        app = subprocess.Popen([str(release), *args, "--snapshot", str(out / "installed-release.png")], env=environment, stdout=log, stderr=log)
        processes.append(app)
        until(lambda: window() is not None)
        window_ms = round((time.monotonic() - started) * 1000)
        until(lambda: prop("CanSeek"))
        result = {"package": str(release.resolve()), "startup_window_ms": window_ms, "startup_to_restored_mpris_ms": round((time.monotonic() - started) * 1000), "renderer": "FemtoVG / Wayland", "audio": "GStreamer autoaudiosink", "interval_seconds": 45}
        time.sleep(4)
        result["paused"] = measure()
        call("Play")
        until(lambda: prop("PlaybackStatus") == "Playing")
        result["playing_visible"] = measure()
        subprocess.run(["niri", "msg", "action", "close-window", "--id", str(window()["id"])], check=True, stdout=subprocess.DEVNULL)
        until(lambda: window() is None)
        result["playing_hidden"] = measure()
        call("Pause")
        call("Raise", "org.mpris.MediaPlayer2")
        until(lambda: window() is not None)
        # Warm pipelines before comparing repeated batches of track switches.
        for batch in range(10):
            for _ in range(30):
                call("Next"); time.sleep(.06)
                call("Previous"); time.sleep(.06)
            # Wait for the final requested seek to finish, draining queued pipeline work.
            target = (7 + batch) * 1_000_000
            def settled():
                if abs(prop("Position") - target) < 100_000:
                    return True
                if prop("CanSeek"):
                    track = prop("Metadata")["mpris:trackid"]["data"]
                    subprocess.run(["busctl", f"--address={address}", "call", name, path, player, "SetPosition", "ox", track, str(target)], check=True, timeout=5, stdout=subprocess.DEVNULL)
                return False
            until(settled, 30)
            time.sleep(2)
            result[f"rss_after_{(batch+1)*60}_part_switches_mib"] = round(sample()[1], 2)
        result["paused_after_stress"] = measure()
        call("Quit", "org.mpris.MediaPlayer2")
        app.wait(timeout=10)
        assert app.returncode == 0
        result["passed"] = True
        report.write_text(json.dumps(result))
finally:
    for process in processes:
        if process.poll() is None:
            process.terminate()
            try: process.wait(timeout=5)
            except subprocess.TimeoutExpired: process.kill(); process.wait(timeout=5)
    os.kill(bus_pid, signal.SIGTERM)
