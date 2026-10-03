#!/usr/bin/env python3
"""Verify real process death and checkpoints in the isolated playback test package."""
import argparse
import io
import json
import pathlib
import re
import sqlite3
import subprocess
import tarfile
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--adb', default='adb')
args = parser.parse_args()
package = 'io.github.mny315.carlitos.playbacktest'
component = package + '/io.github.mny315.carlitos.MainActivity'
service = package + '/io.github.mny315.carlitos.PlaybackService'
output = pathlib.Path('target/tests/android/playback')
output.mkdir(parents=True, exist_ok=True)
report = {}

def adb(*command, binary=False):
    value = subprocess.check_output([args.adb, *map(str, command)], timeout=20)
    return value if binary else value.decode().strip()

def shell(*command):
    return adb('shell', *command)

def pid():
    result = subprocess.run([args.adb, 'shell', 'pidof', package], capture_output=True, text=True, timeout=10)
    assert result.returncode in (0, 1), result.stderr
    return result.stdout.strip()

def state():
    match = re.search(r'CarlitosAudio (\{[^\n]+\})', shell('dumpsys', 'activity', 'service', service))
    return json.loads(match[1]) if match else {}

def until(description, predicate, seconds=10):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(.15)
    raise AssertionError(description + ': ' + json.dumps(state()))

def database(label):
    # Copy DB + WAL together; SQLite ignores any incomplete WAL tail. Reopen a
    # private host copy so the device's DB/locks are never modified by inspection.
    data = adb('exec-out', 'run-as', package, 'tar', '-cf', '-', 'files', binary=True)
    (output / (label + '.tar')).write_bytes(data)
    with tempfile.TemporaryDirectory() as directory:
        root = pathlib.Path(directory)
        with tarfile.open(fileobj=io.BytesIO(data)) as archive:
            for member in archive:
                name = pathlib.PurePosixPath(member.name).name
                if member.isfile() and name in ('library.sqlite3', 'library.sqlite3-wal', 'settings.json'):
                    (root / name).write_bytes(archive.extractfile(member).read())
        with sqlite3.connect(root / 'library.sqlite3') as connection:
            assert connection.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
            saved = json.loads(connection.execute('SELECT data FROM session_state WHERE id=1').fetchone()[0])
        return {'session': saved, 'settings': json.loads((root / 'settings.json').read_text())}

def launch():
    # A short device screen timeout can expire between lifecycle cases. Launch
    # from an unlocked foreground, as a user would, before checking the service.
    shell('input', 'keyevent', 'KEYCODE_WAKEUP')
    shell('wm', 'dismiss-keyguard')
    deadline = time.monotonic() + 90
    waiting = False
    while True:
        policy = shell('dumpsys', 'window', 'policy')
        showing = re.search(r'^\s*showing=(true|false)\s*$', policy, re.MULTILINE)
        if showing and showing[1] == 'false':
            break
        if not waiting:
            print('WAIT unlock the phone before the cold launch (up to 90 seconds)', flush=True)
            waiting = True
        if time.monotonic() >= deadline:
            raise AssertionError('Phone is still locked; unlock it before rerunning the lifecycle test')
        time.sleep(.5)
    shell('am', 'start', '-W', '-n', component)
    until('Ready after launch', lambda: state().get('phase') == 'ready')

def key(action):
    shell('input', 'keyevent', 'KEYCODE_MEDIA_' + action)
    until(action, lambda: state().get('playing') == (action == 'PLAY'))

# Instrumentation can leave a cached process or a pending service restart.
# A cold-restore assertion must start from an explicitly stopped test package.
shell('am', 'force-stop', package)
launch()
initial = state()
assert not initial['playing'] and abs(initial['position'] - 8000) < 150, initial
assert abs(initial['rate'] - 1.75) < .001 and initial['silence'] and abs(initial['volume'] - .25) < .001, initial
report['cold_restore'] = initial
key('PLAY')
previous_pid = pid()
shell('input', 'keyevent', 'KEYCODE_HOME')
shell('input', 'keyevent', 'KEYCODE_SLEEP')
time.sleep(5)
first = database('periodic-1')
time.sleep(5)
second = database('periodic-2')
assert second['session']['position'] > first['session']['position'] + 4000, (first, second)
assert 'isForeground=true' in shell('dumpsys', 'activity', 'services', package)
assert 'mHalInteractiveModeEnabled=false' in shell('dumpsys', 'power')
report['periodic_screen_off'] = [first, second]
print('PASS periodic checkpoints while playing with screen off', flush=True)

# SIGKILL provides no Activity/Service onDestroy or Rust Quit opportunity.
shell('am', 'broadcast', '-n', package + '/io.github.mny315.carlitos.TestProcessReceiver',
      '-a', 'io.github.mny315.carlitos.test.SIGKILL', '--ei', 'expected-pid', previous_pid)
until('Old process gone', lambda: pid() != previous_pid)
crashed = database('after-sigkill')
launch()
restored = state()
assert not restored['playing'] and restored['error'] is None, restored
assert abs(restored['position'] - crashed['session']['position']) < 200, (restored, crashed)
report['sigkill_restore'] = {'saved': crashed, 'restored': restored}
print('PASS SIGKILL restores the last periodic checkpoint paused', flush=True)

key('PLAY')
time.sleep(.7)
key('PAUSE')
time.sleep(.3)
paused = state()
saved = database('pause')
assert abs(saved['session']['position'] - paused['position']) < 200, (saved, paused)
shell('am', 'force-stop', package)
time.sleep(.3)
assert not pid(), 'force-stop left the process running'
launch()
final = state()
assert not final['playing'] and abs(final['position'] - saved['session']['position']) < 200, (final, saved)
assert abs(final['rate'] - 1.75) < .001 and final['silence'] and abs(final['volume'] - .25) < .001, final
report['force_stop_restore'] = {'saved': saved, 'restored': final}
report['device'] = {key: shell('getprop', prop) for key, prop in (
    ('model', 'ro.product.model'), ('android', 'ro.build.version.release'), ('api', 'ro.build.version.sdk'))}
(output / 'lifecycle-result.json').write_text(json.dumps(report, indent=2) + '\n')
print('PASS force-stop stops audio; reopen restores pause, source position, speed, silence and volume', flush=True)
