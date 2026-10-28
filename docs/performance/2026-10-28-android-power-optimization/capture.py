#!/usr/bin/env python3
import argparse
import json
import pathlib
import re
import statistics
import subprocess
import time

parser = argparse.ArgumentParser(description='Compare CPU and whole-device discharge during Android playback.')
parser.add_argument('name')
parser.add_argument('--seconds', type=int, default=90)
parser.add_argument('--screen', choices=['on', 'off'], default='on')
parser.add_argument('--adb', default='adb')
parser.add_argument('--serial')
parser.add_argument('--package', default='io.github.mny315.carlitos')
parser.add_argument('--out', type=pathlib.Path)
args = parser.parse_args()
if args.seconds < 15 or args.seconds % 15:
    parser.error('seconds must be a positive multiple of 15')
if not re.fullmatch(r'[A-Za-z0-9_.]+', args.package):
    parser.error('invalid package')
out = args.out or pathlib.Path('target/power') / args.name
out.mkdir(parents=True, exist_ok=True)
adb = [args.adb] + (['-s', args.serial] if args.serial else [])
package = args.package
component = package + '/io.github.mny315.carlitos.PlaybackService'

def sh(command):
    return subprocess.run(adb + ['shell', command], capture_output=True, text=True,
                          check=True, timeout=20).stdout

def battery_property(raw):
    words = []
    for line in raw.splitlines():
        m = re.match(r'\s*0x[0-9a-f]+:\s+((?:[0-9a-f]{8}\s*)+)', line)
        if m:
            words.extend(int(x, 16) for x in m[1].split())
    assert len(words) == 5 and words[:3] == [0, 0, 1], raw
    value = words[3] | words[4] << 32
    return value - (1 << 64) if value >= 1 << 63 else value

tick_hz = int(sh('getconf CLK_TCK'))
page_bytes = int(sh('getconf PAGESIZE'))

names = [package, 'surfaceflinger', 'audioserver', 'system_server']
command = (f'echo SERVICE; dumpsys activity service {component}; echo UPTIME; cat /proc/uptime; echo CHARGE; '
           'service call batteryproperties 1 i32 1; echo CURRENT; '
           'service call batteryproperties 1 i32 2; '
           + ''.join(f'echo PROCESS:{n}; for p in $(pidof {n}); do cat /proc/$p/stat; done; ' for n in names)
           + "echo MEDIA; dumpsys media_session | grep 'state=PlaybackState'; "
             "echo POWER; dumpsys power | grep -E 'mWakefulness=|mIsPowered='; "
             "echo FOCUS; dumpsys activity activities | grep topResumedActivity; true")

def sample(index):
    raw = sh(command)
    (out / f'{index:02d}.txt').write_text(raw)
    row = {'uptime_s': float(re.search(r'UPTIME\s+([\d.]+)', raw)[1]),
           'charge_uah': battery_property(raw.split('CHARGE\n')[1].split('CURRENT')[0]),
           'current_ua': battery_property(raw.split('CURRENT\n')[1].split('PROCESS:')[0]),
           'processes': {}}
    service = re.search(r'CarlitosAudio (\{[^\n]+\})', raw)
    assert service and json.loads(service[1])['playing'], raw
    assert 'mIsPowered=false' in raw, raw
    if args.screen == 'on':
        assert 'mWakefulness=Awake' in raw, raw
    else:
        display = sh("dumpsys display | grep mScreenState=")
        assert 'mScreenState=OFF' in display, display
    assert 'state=PlaybackState {state=3' in raw, raw
    if args.screen == 'on':
        assert package in raw.split('FOCUS\n')[1], raw
    for n in names:
        section = raw.split('PROCESS:' + n + '\n')[1].split('PROCESS:')[0].split('MEDIA')[0]
        m = re.search(r'^(\d+) \((.*?)\) (.*)$', section, re.M)
        if m:
            fields = m[3].split()
            row['processes'][n] = {'pid': int(m[1]), 'cpu_ticks': int(fields[11]) + int(fields[12]),
                                    'rss_pages': int(fields[21]), 'start_ticks': int(fields[19])}
    assert package in row['processes']
    return row

for suffix, cmd in [('service', f'dumpsys activity service {component}'),
                    ('battery', 'dumpsys battery'),
                    ('settings', 'settings get system screen_brightness; settings get system screen_brightness_mode; cmd media_session volume --stream 3 --get')]:
    (out / f'{suffix}.txt').write_text(sh(cmd))
rows = [sample(0)]
started = time.monotonic()
for i in range(1, args.seconds // 15 + 1):
    time.sleep(max(0, started + 15 * i - time.monotonic()))
    rows.append(sample(i))
    print(args.name, i * 15, 'seconds', flush=True)

a, b = rows[0], rows[-1]
duration = b['uptime_s'] - a['uptime_s']
result = {'name': args.name, 'duration_s': duration, 'tick_hz': tick_hz, 'page_bytes': page_bytes,
          'drain_mah': (a['charge_uah'] - b['charge_uah']) / 1000,
          'device_average_ma': (a['charge_uah'] - b['charge_uah']) * 3.6 / duration,
          'instantaneous_current_median_ma': -statistics.median(r['current_ua'] for r in rows) / 1000,
          'cpu_percent_one_core': {}, 'rss_mib': {}}
for n in names:
    pa, pb = a['processes'][n], b['processes'][n]
    assert pa['pid'] == pb['pid'] and pa['start_ticks'] == pb['start_ticks']
    result['cpu_percent_one_core'][n] = (pb['cpu_ticks'] - pa['cpu_ticks']) * 100 / tick_hz / duration
    result['rss_mib'][n] = pb['rss_pages'] * page_bytes / 1048576
(out / 'samples.json').write_text(json.dumps(rows, indent=2))
(out / 'summary.json').write_text(json.dumps(result, indent=2))
for suffix, cmd in [('battery-after', 'dumpsys battery'), ('meminfo', f'dumpsys meminfo {package}'), ('layers', 'dumpsys SurfaceFlinger --list')]:
    (out / f'{suffix}.txt').write_text(sh(cmd))
layer_names = [line for line in (out / 'layers.txt').read_text().splitlines()
               if package in line and ('SurfaceView' in line or 'MainActivity' in line)]
for i, name in enumerate(layer_names):
    (out / f'latency-{i}.txt').write_text(sh('dumpsys SurfaceFlinger --latency ' + "'" + name.replace("'", "'\\''") + "'"))
image = subprocess.run(adb + ['exec-out', 'screencap', '-p'], capture_output=True, check=True, timeout=20)
(out / 'playing.png').write_bytes(image.stdout)
print(json.dumps(result, indent=2), flush=True)
