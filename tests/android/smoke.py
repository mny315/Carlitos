#!/usr/bin/env python3
"""Check a selected, playing Android document through real system controls.

Requires an unlocked ADB device and a recording with at least --seconds + 15
seconds remaining. Leaves playback paused. Artifacts stay outside source files.
"""
import argparse
import json
import pathlib
import re
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--adb', default='adb')
parser.add_argument('--seconds', type=int, default=60)
parser.add_argument('--output', type=pathlib.Path, default=pathlib.Path('target/tests/android/session'))
args = parser.parse_args()
assert args.seconds >= 1
args.output.mkdir(parents=True, exist_ok=True)
package = 'io.github.mny315.carlitos'
component = package + '/.MainActivity'

def adb(*command):
    return subprocess.check_output([args.adb, *map(str, command)], text=True, timeout=20).strip()

def shell(*command):
    return adb('shell', *command)

def state():
    result = shell('dumpsys', 'activity', 'service', package + '/.PlaybackService')
    return json.loads(re.search(r'CarlitosAudio (\{[^\n]+\})', result)[1])

def capture(name):
    (args.output / (name + '.png')).write_bytes(subprocess.check_output(
        [args.adb, 'exec-out', 'screencap', '-p'], timeout=15))

def wait_playing(value):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        snapshot = state()
        if snapshot['playing'] == value:
            return snapshot
        time.sleep(.1)
    raise AssertionError(snapshot)

shell('input', 'keyevent', 'KEYCODE_WAKEUP')
shell('wm', 'dismiss-keyguard')
shell('am', 'start', '-W', '-n', component)
deadline = time.monotonic() + 8
while True:
    try:
        initial = state()
        if initial['phase'] == 'ready':
            break
    except (TypeError, subprocess.CalledProcessError):
        pass
    assert time.monotonic() < deadline, 'Media session did not become ready'
    time.sleep(.1)
if not initial['playing']:
    shell('input', 'keyevent', 'KEYCODE_MEDIA_PLAY')
    initial = wait_playing(True)
assert initial['playing'] and initial['error'] is None, initial
pid = shell('pidof', package)
report = {'model': shell('getprop', 'ro.product.model'),
          'android': shell('getprop', 'ro.build.version.release'),
          'api': shell('getprop', 'ro.build.version.sdk'), 'pid': pid, 'initial': initial}
capture('playing')
services = shell('dumpsys', 'activity', 'services', package)
assert 'isForeground=true' in services, services
(args.output / 'foreground-service.txt').write_text(services)
shell('input', 'keyevent', 'KEYCODE_HOME')
shell('input', 'keyevent', 'KEYCODE_SLEEP')
started = time.monotonic()
before = state()
samples = []
while time.monotonic() - started < args.seconds:
    time.sleep(min(15, args.seconds - (time.monotonic() - started)))
    sample = state()
    assert sample['playing'] and sample['error'] is None, sample
    assert sample['player'] == initial['player'], sample
    power = shell('dumpsys', 'power')
    assert 'mHalInteractiveModeEnabled=false' in power, power
    display = shell('dumpsys', 'display')
    assert 'mScreenState=OFF' in display, display
    samples.append(sample)
    print(json.dumps({'screen_off_seconds': round(time.monotonic()-started), **sample}), flush=True)
elapsed = time.monotonic() - started
assert samples[-1]['position'] - before['position'] >= (elapsed - 3) * 1000, samples
report.update(screen_off_seconds=elapsed, before_screen_off=before, samples=samples)
capture('screen-off')
shell('input', 'keyevent', 'KEYCODE_MEDIA_PAUSE')
paused = wait_playing(False)
time.sleep(.5)
assert abs(state()['position'] - paused['position']) < 200
shell('input', 'keyevent', 'KEYCODE_MEDIA_PLAY')
wait_playing(True)
shell('input', 'keyevent', 'KEYCODE_WAKEUP')
time.sleep(.3)
shell('wm', 'dismiss-keyguard')
shell('am', 'start', '-W', '-n', component)
time.sleep(1)
assert shell('pidof', package) == pid
assert state()['player'] == initial['player']
capture('returned')
# Exercise Back/reopening separately from instrumentation Activity.recreate().
shell('input', 'keyevent', 'KEYCODE_BACK')
time.sleep(.5)
assert state()['playing']
shell('am', 'start', '-W', '-n', component)
time.sleep(1)
assert state()['player'] == initial['player']
threads = shell('run-as', package, 'sh', '-c', '"cat /proc/' + pid + '/task/*/comm"')
report['controller_threads'] = threads.splitlines().count('carlitos-contro')
assert report['controller_threads'] == 1, threads
report['after_activity_reopen'] = state()
capture('reopened')
shell('input', 'keyevent', 'KEYCODE_MEDIA_PAUSE')
report['final_paused'] = wait_playing(False)
logs = adb('logcat', '-d', '--pid=' + pid)
(args.output / 'device-logcat.txt').write_text(logs)
for failure in ('FATAL EXCEPTION', 'panicked at', 'Fatal signal', 'Carlitos Android startup:'):
    assert failure not in logs, failure
(args.output / 'background-result.json').write_text(json.dumps(report, indent=2) + '\n')
print('PASS screen-off playback, system controls, Home/Back/reopen and one controller/player', flush=True)
