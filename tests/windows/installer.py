#!/usr/bin/env python3
"""Exercise the Windows release installer in a private Wine prefix (needs X11)."""
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import time


ROOT = pathlib.Path(__file__).resolve().parents[2]
SETUP = ROOT / "target/releases/Carlitos.exe"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check(prefix, env, log):
    def run(*args):
        subprocess.run(["wine", *map(str, args)], env=env, stdout=log,
                       stderr=subprocess.STDOUT, timeout=90, check=True)

    def install():
        run(SETUP, "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART",
            "/TASKS=desktopicon")

    install()
    exe = next((prefix / "drive_c/users").glob(
        "*/AppData/Local/Programs/Carlitos/Carlitos.exe"))
    folder = exe.parent
    assert (folder / "carlitos-installed").is_file()
    assert (folder / "licenses.txt").stat().st_size > 100
    links = list(exe.parents[4].rglob("Carlitos.lnk"))
    assert len(links) == 2, links
    # Use the actual default renderer. This catches the Slint zero-refresh-rate
    # startup panic under Xvfb, which offscreen/software-only tests missed.
    run(exe, "--no-desktop", "--language", "ru", "--quit-after", "3")
    data = folder.parents[1] / "Carlitos"
    settings, database = data / "settings.json", data / "library.sqlite3"
    assert settings.is_file() and database.is_file(), data
    assert not (folder / "Carlitos-data").exists()
    preferences = json.loads(settings.read_text())
    preferences["theme"] = "light"
    settings.write_text(json.dumps(preferences, ensure_ascii=False))
    before = [digest(settings), digest(database)]
    install()
    assert before == [digest(settings), digest(database)], "Upgrade changed data"
    run(exe, "--no-desktop", "--quit-after", "3")
    assert json.loads(settings.read_text())["theme"] == "light"
    portable = prefix / "drive_c/Portable audit"
    portable.mkdir()
    shutil.copyfile(exe, portable / "Carlitos.exe")
    run(portable / "Carlitos.exe", "--no-desktop", "--quit-after", "3")
    assert (portable / "Carlitos-data/library.sqlite3").is_file()
    assert (portable / "Carlitos-data/settings.json").is_file()
    before = [digest(settings), digest(database)]
    run(folder / "unins000.exe", "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART")
    deadline = time.monotonic() + 20
    while folder.exists() and time.monotonic() < deadline:
        time.sleep(.1)
    assert not folder.exists(), "Uninstaller left the installation directory"
    assert all(not link.exists() for link in links), "Uninstaller left shortcuts"
    assert before == [digest(settings), digest(database)], "Uninstaller changed data"
    registry = subprocess.run([
        "wine", "reg", "query",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\io.github.mny315.Carlitos_is1",
    ], env=env, stdout=log, stderr=subprocess.STDOUT, timeout=20)
    assert registry.returncode != 0, "Uninstaller left its registration"


def main():
    assert SETUP.is_file(), "Build the installer first: ./build.sh windows"
    output = ROOT / "target/tests/windows-installer"
    output.mkdir(parents=True, exist_ok=True)
    result_path = output / "result.json"
    result = {"passed": False, "installer_sha256": digest(SETUP)}
    # Replace a previous success before starting a potentially failing run.
    result_path.write_text(json.dumps(result, indent=2))
    with tempfile.TemporaryDirectory(prefix="wine-", dir=output) as temporary:
        prefix = pathlib.Path(temporary)
        env = dict(os.environ, WINEPREFIX=str(prefix), WINEARCH="win64",
                   WINEDEBUG="-all", WINEDLLOVERRIDES="mscoree,mshtml=d")
        for key in ("SLINT_BACKEND", "SLINT_DEFAULT_FONT", "SLINT_FONT_PATH"):
            env.pop(key, None)
        with (output / "wine.log").open("w") as log:
            try:
                check(prefix, env, log)
            finally:
                subprocess.run(["wineserver", "-k"], env=env, timeout=20)
                subprocess.run(["wineserver", "-w"], env=env, timeout=20)
    result.update(passed=True, checks=[
        "silent install and shortcuts", "startup with default renderer",
        "installed data location", "upgrade preserves settings and SQLite",
        "portable copy uses adjacent data", "uninstall preserves user data",
    ])
    result_path.write_text(json.dumps(result, indent=2))
    print(json.dumps(result))


if __name__ == "__main__":
    main()
