#!/usr/bin/env python3
"""Exercise the real UI/controller/MPRIS path on a private bus and temporary data."""
import json
import math
import os
from pathlib import Path
import signal
import sqlite3
import struct
import subprocess
import tempfile
import time
import wave
import sys

root = Path(__file__).resolve().parents[2]
binary = Path(os.environ.get("CARGO_TARGET_DIR", root / "target")) / "debug/carlitos"
real_audio = "--real-audio" in sys.argv
output = root / "target/tests" / ("desktop-real-audio" if real_audio else "desktop")
output.mkdir(parents=True, exist_ok=True)
(output / "result.json").unlink(missing_ok=True)
# No service directories: never activate desktop services from the user's session.
config = output / "bus.conf"
config.write_text('<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>')
bus = subprocess.check_output(["dbus-daemon", f"--config-file={config}", "--fork", "--print-address=1", "--print-pid=1"], text=True).splitlines()
address, bus_pid = bus[0], int(bus[1])
environment = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=address)
name, path = "org.mpris.MediaPlayer2.Carlitos", "/org/mpris/MediaPlayer2"
player = "org.mpris.MediaPlayer2.Player"
processes = []

def call(method, signature=None, *args, interface=player, destination=name):
    command = ["busctl", f"--address={address}", "call", destination, path, interface, method]
    if signature:
        command += [signature, *map(str, args)]
    return subprocess.check_output(command, text=True, stderr=subprocess.STDOUT, timeout=5)

def prop(property_name, destination=name):
    result = json.loads(subprocess.check_output(["busctl", f"--address={address}", "--json=short", "get-property", destination, path, player, property_name], text=True, stderr=subprocess.DEVNULL, timeout=5))
    return result["data"]

def eventually(condition, description, seconds=12):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        try:
            if condition():
                print(f"PASS: {description}", flush=True)
                return
        except (subprocess.CalledProcessError, KeyError):
            pass
        time.sleep(.05)
    raise AssertionError(description)

try:
    with tempfile.TemporaryDirectory(prefix="carlitos-desktop-") as temporary:
        folder = Path(temporary) / "Аудиокнига с пробелами"
        folder.mkdir()
        frames = b"".join(struct.pack("<h", int(1200 * math.sin(i * 2 * math.pi * 440 / 8000))) for i in range(12 * 8000))
        for part in (1, 2, 10):
            with wave.open(str(folder / f"{part}.wav"), "wb") as audio:
                audio.setnchannels(1)
                audio.setsampwidth(2)
                audio.setframerate(8000)
                audio.writeframes(frames)
        (folder / "broken.mp3").write_bytes(b"not audio")
        data = Path(temporary) / "data"
        environment["CARLITOS_TEST_SOURCE_FOLDER"] = str(folder)
        args = [str(binary), "--data-dir", str(data), "--language", "ru"]
        if not real_audio:
            args += ["--fake-audio"]
        log_path = output / "scenario.log"
        with log_path.open("w") as log:
            app = subprocess.Popen([*args, "--exercise", str(folder)], env=environment, stdout=log, stderr=log)
            processes.append(app)
            eventually(lambda: "UI SCENARIO READY" in log_path.read_text(), "folder preview → import → playback → seek → pause through UI", 45)
            eventually(lambda: json.loads((data / "settings.json").read_text()).get("library_tab") == "started", "visiting completed books preserves the started tab on disk")
            assert app.poll() is None
            eventually(lambda: prop("PlaybackStatus") == "Paused", "MPRIS sees paused player")
            position = prop("Position")
            assert 2_800_000 <= position <= 4_200_000, position
            metadata = prop("Metadata")
            (output / "metadata.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2))
            track = metadata["mpris:trackid"]["data"]
            assert prop("CanGoNext") and not prop("CanGoPrevious") and prop("CanSeek")
            assert prop("MinimumRate") == 0.5 and prop("MaximumRate") == 3.0
            call("Set", "ssv", player, "Volume", "d", .37, interface="org.freedesktop.DBus.Properties")
            eventually(lambda: prop("Volume") == .37, "MPRIS changes volume without replacing track metadata")
            assert prop("Metadata") == metadata
            assert abs(prop("Position") - position) < 150_000
            call("Set", "ssv", player, "Rate", "d", 1.75, interface="org.freedesktop.DBus.Properties")
            eventually(lambda: prop("Rate") == 1.75, "MPRIS changes playback speed")
            assert prop("PlaybackStatus") == "Paused"
            assert abs(prop("Position") - position) < 150_000
            call("Play")
            eventually(lambda: prop("Position") > position + 300_000, "MPRIS Play advances audio")
            call("Pause")
            eventually(lambda: prop("PlaybackStatus") == "Paused", "MPRIS Pause")
            for target in (2_000_000, 8_000_000, 4_000_000):
                call("SetPosition", "ox", track, target)
            eventually(lambda: abs(prop("Position") - 4_000_000) < 150_000, "rapid MPRIS SetPosition keeps final request")
            call("Next")
            eventually(lambda: prop("Metadata")["mpris:trackid"]["data"] != track, "next part metadata changes")
            assert prop("Rate") == 1.75
            assert prop("PlaybackStatus") == "Paused"
            call("Previous")
            eventually(lambda: prop("Metadata")["mpris:trackid"]["data"] == track, "previous part")
            # Metadata changes when loading starts; SetPosition is accepted
            # only once MPRIS advertises that the new part is seekable.
            eventually(lambda: prop("CanSeek"), "previous part is ready for seeking")
            call("SetPosition", "ox", track, 4_000_000)
            eventually(lambda: abs(prop("Position") - 4_000_000) < 150_000, "seek before crash test")
            call("Play")
            eventually(lambda: prop("Position") >= 8_200_000, "periodic playback checkpoint", 8)
            owner_before = subprocess.check_output(["busctl", f"--address={address}", "call", "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "GetNameOwner", "s", name], text=True)
            second = subprocess.run(args, env=environment, stdout=log, stderr=log, timeout=8)
            assert second.returncode == 0
            owner_after = subprocess.check_output(["busctl", f"--address={address}", "call", "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "GetNameOwner", "s", name], text=True)
            assert owner_before == owner_after
            print("PASS: second launch activates the existing process", flush=True)
            app.kill()
            app.wait(timeout=8)
            database = sqlite3.connect(data / "library.sqlite3")
            assert database.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
            saved = json.loads(database.execute("SELECT data FROM session_state WHERE id=1").fetchone()[0])
            database.close()
            assert saved["position"] >= 4_000, saved
            app = subprocess.Popen([*args, "--test-library-tab", "1"], env=environment, stdout=log, stderr=log)
            processes.append(app)
            eventually(lambda: prop("PlaybackStatus") == "Paused" and abs(prop("Position") - saved["position"] * 1000) < 200_000, "crash recovery restores confirmed position on pause")
            eventually(lambda: prop("Rate") == 1.75, "playback speed survives restart")
            eventually(lambda: "UI TAB RESTORED 1" in log_path.read_text(), "started tab and its filtered model survive a process restart")
            eventually(lambda: json.loads((data / "settings.json").read_text()).get("library_tab") == "all", "all tab is remembered after visiting completed books")
            call("Set", "ssv", player, "Rate", "d", 1.0, interface="org.freedesktop.DBus.Properties")
            eventually(lambda: prop("Rate") == 1.0, "reset playback speed")
            call("Quit", interface="org.mpris.MediaPlayer2")
            app.wait(timeout=10)
            assert app.returncode == 0
            print("PASS: MPRIS Quit saves and joins workers", flush=True)
            def visible(process=None):
                windows = json.loads(subprocess.check_output(["niri", "msg", "-j", "windows"]))
                return any(w.get("pid") == (process or app).pid for w in windows)
            # The real window close callback, including the safe no-host fallback.
            app = subprocess.Popen([*args, "--test-library-tab", "0", "--test-close-after", "2", "--snapshot", str(output / "no-tray.png")], env=environment, stdout=log, stderr=log)
            processes.append(app)
            eventually(lambda: prop("CanPlay"), "no-host instance is ready")
            eventually(lambda: "UI TAB RESTORED 0" in log_path.read_text(), "all tab survives restart after closing on completed books")
            time.sleep(3.5)
            assert visible() and app.poll() is None
            print("PASS: closing without a tray host keeps a recoverable window", flush=True)
            call("Quit", interface="org.mpris.MediaPlayer2")
            app.wait(timeout=10)

            def host():
                process = subprocess.Popen([str(binary), "--mock-tray-host"], env=environment, stdout=log, stderr=log)
                processes.append(process)
                eventually(lambda: subprocess.run(["busctl", f"--address={address}", "get-property", "org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher", "org.kde.StatusNotifierWatcher", "IsStatusNotifierHostRegistered"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0, "private test tray host owns its name")
                return process
            watcher = host()
            app = subprocess.Popen([*args, "--test-close-after", "3,10"], env=environment, stdout=log, stderr=log)
            processes.append(app)
            eventually(lambda: prop("CanSeek"), "tray instance is ready")
            items = json.loads(subprocess.check_output(["busctl", f"--address={address}", "--json=short", "get-property", "org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher", "org.kde.StatusNotifierWatcher", "RegisteredStatusNotifierItems"], text=True))["data"]
            service = items[-1].removesuffix("/StatusNotifierItem")
            def tray_property(name):
                return json.loads(subprocess.check_output(["busctl", f"--address={address}", "--json=short", "get-property", service, "/StatusNotifierItem", "org.kde.StatusNotifierItem", name], text=True))["data"]
            assert tray_property("IconName") == "", "an installed icon must not override the embedded artwork"
            assert tray_property("IconPixmap"), "embedded tray artwork is present"
            print("PASS: tray exposes embedded artwork without a stale theme icon name", flush=True)
            call("SetPosition", "ox", track, 1_000_000)
            call("Play")
            eventually(lambda: not visible(), "close hides the window only after confirmed tray registration", 7)
            position = prop("Position")
            eventually(lambda: prop("Position") > position + 300_000, "hidden player continues audio")
            watcher.terminate()
            watcher.wait(timeout=5)
            eventually(visible, "tray host loss restores the hidden window", 4)
            watcher = host()
            eventually(lambda: not visible(), "tray host restart allows hiding again", 12)
            call("Raise", interface="org.mpris.MediaPlayer2")
            eventually(visible, "MPRIS Raise restores a hidden window")
            # A separate data directory owns its own player and activation route.
            other_args = [str(binary), "--data-dir", str(Path(temporary) / "other-data"), "--fake-audio"]
            other = subprocess.Popen([*other_args, "--test-close-after", "3"], env=environment, stdout=log, stderr=log)
            processes.append(other)
            other_name = f"{name}.instance{other.pid}"
            eventually(lambda: prop("CanControl", other_name), "separate libraries have independent MPRIS services")
            assert prop("CanPlay") and not prop("CanPlay", other_name)
            original_volume = prop("Volume")
            call("Set", "ssv", player, "Volume", "d", .25, interface="org.freedesktop.DBus.Properties", destination=other_name)
            eventually(lambda: prop("Volume", other_name) == .25, "secondary player's controls reach its library")
            assert prop("Volume") == original_volume
            eventually(lambda: not visible(other), "secondary library can hide in the tray", 7)
            duplicate = subprocess.run(other_args, env=environment, stdout=log, stderr=log, timeout=8)
            assert duplicate.returncode == 0
            eventually(lambda: visible(other), "second launch restores the matching library's window")
            call("Quit", interface="org.mpris.MediaPlayer2", destination=other_name)
            other.wait(timeout=10)
            assert other.returncode == 0 and app.poll() is None
            call("Quit", interface="org.mpris.MediaPlayer2")
            app.wait(timeout=10)
            assert app.returncode == 0
            for mode in ("error", "cancel", "source"):
                if mode == "cancel":
                    portal = subprocess.Popen([str(binary), "--mock-portal"], env=environment, stdout=log, stderr=log)
                    processes.append(portal)
                    eventually(lambda: "TEST PORTAL READY" in log_path.read_text(), "private portal fixture is ready")
                previous = log_path.read_text().count("UI PORTAL READY")
                app = subprocess.Popen([*args, "--test-portal", mode], env=environment, stdout=log, stderr=log)
                processes.append(app)
                eventually(lambda: log_path.read_text().count("UI PORTAL READY") > previous, f"portal {mode}: repeated requests leave UI responsive")
                if mode == "cancel":
                    assert log_path.read_text().count("TEST PORTAL PARENT wayland:") == 2, log_path.read_text()
                    print("PASS: portal receives an exported Wayland parent for both requests", flush=True)
                call("Quit", interface="org.mpris.MediaPlayer2")
                app.wait(timeout=10)
                assert app.returncode == 0
            app = subprocess.Popen([*args, "--test-theme"], env=environment, stdout=log, stderr=log)
            processes.append(app)
            eventually(lambda: "UI THEME DARK" in log_path.read_text(), "system theme reads portal preference")
            def scheme(value):
                subprocess.run(["busctl", f"--address={address}", "call", "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings", "SetScheme", "u", str(value)], check=True, timeout=5)
            scheme(2)
            eventually(lambda: "UI THEME MANUAL" in log_path.read_text(), "system theme follows live portal changes")
            scheme(1)
            scheme(2)
            eventually(lambda: "UI THEME READY" in log_path.read_text(), "manual theme remains independent of system changes")
            call("Quit", interface="org.mpris.MediaPlayer2")
            app.wait(timeout=10)
            assert app.returncode == 0
            broken_environment = dict(environment, GST_PLUGIN_SYSTEM_PATH_1_0="", GST_PLUGIN_PATH_1_0="", GST_REGISTRY=str(Path(temporary) / "empty-registry.bin"))
            app = subprocess.Popen(args, env=broken_environment, stdout=log, stderr=log)
            processes.append(app)
            eventually(lambda: prop("CanPlay"), "library remains available with missing audio plugins")
            time.sleep(.5)
            assert not prop("CanSeek")
            call("Quit", interface="org.mpris.MediaPlayer2")
            app.wait(timeout=10)
            assert app.returncode == 0
            print("PASS: audio initialization failure cannot hang shutdown", flush=True)
        (output / "result.json").write_text(json.dumps({"passed": True, "saved_position_ms": saved["position"], "real_audio": real_audio}))
finally:
    for app in processes:
        if app.poll() is None:
            app.terminate()
            try:
                app.wait(timeout=5)
            except subprocess.TimeoutExpired:
                app.kill()
                app.wait(timeout=5)
    os.kill(bus_pid, signal.SIGTERM)
