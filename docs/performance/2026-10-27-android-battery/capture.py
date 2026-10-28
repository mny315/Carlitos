#!/usr/bin/env python3
"""Capture a fixed four-mode battery comparison without resetting batterystats.

Prerequisites: an installed app with a long local track already selected, an
unlocked device, Wi-Fi ADB, and no charger. Output stays beside this script.
The final control force-stops the target package; playback finishes paused.
"""
import argparse
import csv
import datetime
import json
import pathlib
import re
import subprocess
import statistics
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--adb', required=True)
    parser.add_argument('--serial', required=True)
    parser.add_argument('--package', default='io.github.mny315.carlitos')
    parser.add_argument('--duration', type=int, default=240)
    parser.add_argument('--interval', type=int, default=15)
    parser.add_argument('--settle', type=int, default=20)
    parser.add_argument('--output', type=pathlib.Path)
    parser.add_argument('--resume', action='store_true', help='Keep completed phases in the output directory')
    args = parser.parse_args()
    out = (args.output or pathlib.Path(__file__).resolve().parent).resolve()
    out.mkdir(parents=True, exist_ok=True)
    if (out / 'samples.jsonl').exists() and not args.resume:
        raise SystemExit('Output already contains samples; choose a new --output directory')
    adb = [args.adb, '-s', args.serial]

    def shell(command, timeout=30):
        p = subprocess.run(adb + ['shell', command], capture_output=True,
                           text=True, timeout=timeout, check=True)
        return p.stdout + p.stderr

    def save_shell(name, command):
        value = shell(command)
        (out / name).write_text(value)
        return value

    def snapshot(phase, edge):
        for name, command in {
            'batterystats': f'dumpsys batterystats --charged {args.package}',
            'power': 'dumpsys power',
            'media': 'dumpsys media_session',
            'cpuinfo': f'dumpsys cpuinfo {args.package}',
            'threads': f'p=$(pidof {args.package}); if [ -n "$p" ]; then '
                       'for f in /proc/$p/task/*/stat; do cat "$f"; done; fi; true',
        }.items():
            save_shell(f'{phase}-{edge}-{name}.txt', command)

    processes = [args.package, 'surfaceflinger', 'audioserver', 'system_server', 'adbd']
    sample_command = (
        'echo UPTIME; cat /proc/uptime; echo BATTERY; dumpsys battery; '
        'echo PROPERTY_CHARGE; service call batteryproperties 1 i32 1; '
        'echo PROPERTY_CURRENT; service call batteryproperties 1 i32 2; '
        'echo PROPERTIES_END; '
        + ''.join(f'echo PROCESS:{name}; for p in $(pidof {name}); do '
                  'cat /proc/$p/stat; done; ' for name in processes)
        + 'echo POWER; dumpsys power | grep -E '
          "'mWakefulness=|mIsPowered=|Wake Locks:|PARTIAL_WAKE_LOCK'; "
          'echo MEDIA; dumpsys media_session | grep -E '
          "'package=io.github.mny315.carlitos$|state=PlaybackState'; "
          'echo FOCUS; dumpsys activity activities | grep -E '
          "'topResumedActivity=|mResumedActivity:'; true"
    )

    def sample(phase, expected_screen, expected_playing, expected_app, index):
        started = time.monotonic()
        raw = shell(sample_command)
        completed = time.monotonic()
        (out / f'{phase}-sample-{index:03d}.txt').write_text(raw)
        r = {
            'phase': phase, 'sample': index,
            'utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
            'host_monotonic_s': (started + completed) / 2,
            'sample_latency_s': completed - started,
            'expected_screen': expected_screen,
            'expected_playing': expected_playing,
            'expected_app_running': expected_app,
        }
        r['device_uptime_s'] = float(re.search(r'UPTIME\s+([\d.]+)', raw)[1])
        for source, dest in [('Charge counter', 'cached_charge_uah'), ('level', 'level_pct'),
                             ('voltage', 'voltage_mv'), ('temperature', 'temperature_deci_c')]:
            r[dest] = int(re.search(r'^\s*' + source + r':\s*(-?\d+)', raw, re.M)[1])
        # Android 13 IBatteryPropertiesRegistrar.getProperty transaction 1:
        # exception, status, parcelable-present, signed int64 (low, high).
        # Query the Health HAL rather than BatteryService's cached broadcast.
        for marker, dest in [('PROPERTY_CHARGE', 'charge_uah'), ('PROPERTY_CURRENT', 'current_ua')]:
            parcel = raw.split(marker + '\n', 1)[1].split('\n' + ('PROPERTY_CURRENT' if marker == 'PROPERTY_CHARGE' else 'PROPERTIES_END'), 1)[0]
            words = []
            for line in parcel.splitlines():
                m = re.match(r'\s*0x[0-9a-f]+:\s+((?:[0-9a-f]{8}\s*)+)', line)
                if m:
                    words.extend(int(x, 16) for x in m[1].split())
            if len(words) != 5 or words[:3] != [0, 0, 1]:
                raise RuntimeError(f'Unsupported battery property {dest}: {parcel}')
            value = words[3] | (words[4] << 32)
            r[dest] = value - (1 << 64) if value >= 1 << 63 else value
        r['wakefulness'] = re.search(r'mWakefulness=(\w+)', raw)[1]
        if expected_screen == 'off':
            display = shell("dumpsys display | grep -E 'mScreenState=|mScreenBrightness='")
            (out / f'{phase}-sample-{index:03d}-display.txt').write_text(display)
            r['display_state'] = re.search(r'mScreenState=(\w+)', display)[1]
        r['charging'] = bool(re.search(r'(?:AC|USB|Wireless) powered: true', raw))
        media = re.findall(r'state=PlaybackState \{state=(\d+), position=(\d+).*?speed=([\d.]+)', raw)
        r['media_state'] = int(media[-1][0]) if media else None
        r['media_position_ms'] = int(media[-1][1]) if media else None
        r['media_speed'] = float(media[-1][2]) if media else None
        r['partial_wakelocks'] = re.findall(r'^.*PARTIAL_WAKE_LOCK.*$', raw, re.M)
        r['focus'] = '\n'.join(line.strip() for line in raw.splitlines()
                               if 'ResumedActivity' in line)
        r['processes'] = {}
        for name in processes:
            section = raw.split('PROCESS:' + name + '\n', 1)[1].split('PROCESS:', 1)[0].split('POWER', 1)[0]
            for line in section.splitlines():
                match = re.match(r'(\d+) \((.*?)\) (.*)', line)
                if not match:
                    continue
                fields = match[3].split()
                r['processes'][name] = {
                    'pid': int(match[1]), 'comm': match[2],
                    'cpu_ticks': int(fields[11]) + int(fields[12]),
                    'user_ticks': int(fields[11]), 'system_ticks': int(fields[12]),
                    'thread_count': int(fields[17]),
                    'start_ticks': int(fields[19]), 'rss_pages': int(fields[21]),
                }
        actual_app = args.package in r['processes']
        errors = []
        if r['charging']:
            errors.append('charger connected')
        if actual_app != expected_app:
            errors.append(f'app running={actual_app}, expected={expected_app}')
        if expected_screen == 'on' and r['wakefulness'] != 'Awake':
            errors.append('screen is not awake')
        if expected_screen == 'off' and r['display_state'] != 'OFF':
            errors.append(f'display state is {r["display_state"]}, expected OFF')
        if expected_app and (r['media_state'] == 3) != expected_playing:
            errors.append(f'media state={r["media_state"]}, expected playing={expected_playing}')
        if expected_screen == 'on' and args.package not in r['focus']:
            errors.append('target activity is not foreground')
        r['validation_errors'] = errors
        with (out / 'samples.jsonl').open('a') as f:
            f.write(json.dumps(r) + '\n')
        if errors:
            raise RuntimeError(f'{phase}: {errors}')
        return r

    settings = {
        key: shell(f'settings get system {key}').strip()
        for key in ['screen_brightness', 'screen_brightness_mode', 'screen_off_timeout']
    }
    settings['music_volume_dump'] = shell('cmd media_session volume --stream 3 --get')
    settings['clock_ticks_per_second'] = int(shell('getconf CLK_TCK').strip())
    settings['page_size_bytes'] = int(shell('getconf PAGESIZE').strip())
    settings['parameters'] = {k: str(v) if isinstance(v, pathlib.Path) else v for k, v in vars(args).items()}
    (out / 'measurement-settings.json').write_text(json.dumps(settings, indent=2))
    phases = [
        ('paused_screen_on', 'on', False, True),
        ('playing_screen_on', 'on', True, True),
        ('playing_screen_off', 'off', True, True),
        ('paused_screen_off', 'off', False, True),
        ('stopped_screen_off_control', 'off', False, False),
    ]
    summary = json.loads((out / 'summary.json').read_text()) if args.resume else []
    completed_phases = {r['phase'] for r in summary}
    try:
        shell('settings put system screen_off_timeout 1800000')
        for phase, screen, playing, app_running in phases:
            if phase in completed_phases:
                continue
            print(f'PREPARE {phase}', flush=True)
            if app_running:
                shell('cmd media_session dispatch ' + ('play' if playing else 'pause'))
            else:
                shell(f'am force-stop {args.package}')
            if screen == 'on':
                shell(f'input keyevent KEYCODE_WAKEUP; wm dismiss-keyguard; '
                      f'am start -n {args.package}/.MainActivity')
            else:
                shell('input keyevent KEYCODE_SLEEP')
            time.sleep(args.settle if app_running else max(args.settle, 45))
            snapshot(phase, 'start')
            first = sample(phase, screen, playing, app_running, 0)
            print(f'START {phase} charge={first["charge_uah"]} uAh, '
                  f'level={first["level_pct"]}%', flush=True)
            phase_start = time.monotonic()
            rows = [first]
            sample_count = args.duration // args.interval
            for i in range(1, sample_count + 1):
                time.sleep(max(0, phase_start + i * args.interval - time.monotonic()))
                r = sample(phase, screen, playing, app_running, i)
                rows.append(r)
                print(f'{phase} {i * args.interval}s: charge={r["charge_uah"]} uAh, '
                      f'temp={r["temperature_deci_c"] / 10:.1f} C, '
                      f'media={r["media_state"]}', flush=True)
            last = rows[-1]
            elapsed = last['device_uptime_s'] - first['device_uptime_s']
            drain_mah = (first['charge_uah'] - last['charge_uah']) / 1000
            result = {'phase': phase, 'duration_s': elapsed, 'sample_count': len(rows),
                      'charge_start_uah': first['charge_uah'], 'charge_end_uah': last['charge_uah'],
                      'drain_mah': drain_mah, 'device_average_ma': drain_mah * 3600 / elapsed,
                      'device_sampled_current_median_ma': -statistics.median(r['current_ua'] for r in rows) / 1000,
                      'device_sampled_current_mean_ma': -statistics.mean(r['current_ua'] for r in rows) / 1000,
                      'level_start_pct': first['level_pct'], 'level_end_pct': last['level_pct'],
                      'temperature_start_c': first['temperature_deci_c'] / 10,
                      'temperature_end_c': last['temperature_deci_c'] / 10,
                      'process_cpu_pct_one_core': {}, 'valid': True}
            for name in processes:
                p0 = first['processes'].get(name)
                p1 = last['processes'].get(name)
                if p0 and p1 and p0['pid'] == p1['pid'] and p0['start_ticks'] == p1['start_ticks']:
                    result['process_cpu_pct_one_core'][name] = (
                        (p1['cpu_ticks'] - p0['cpu_ticks']) / settings['clock_ticks_per_second'] / elapsed * 100)
            snapshot(phase, 'end')
            summary.append(result)
            (out / 'summary.json').write_text(json.dumps(summary, indent=2))
            print('RESULT ' + json.dumps(result), flush=True)
    finally:
        try:
            shell('cmd media_session dispatch pause; input keyevent KEYCODE_SLEEP')
            shell('settings put system screen_off_timeout ' + settings['screen_off_timeout'])
        except Exception as failure:
            print('CLEANUP ERROR: ' + str(failure), flush=True)

    all_rows = [json.loads(line) for line in (out / 'samples.jsonl').read_text().splitlines()]
    columns = ['phase', 'sample', 'utc', 'device_uptime_s', 'charge_uah', 'level_pct',
               'cached_charge_uah', 'current_ua', 'voltage_mv', 'temperature_deci_c', 'wakefulness', 'display_state', 'media_state',
               'media_position_ms', 'sample_latency_s']
    with (out / 'samples.csv').open('w', newline='') as f:
        writer = csv.DictWriter(f, fieldnames=columns)
        writer.writeheader()
        for row in all_rows:
            writer.writerow({key: row.get(key) for key in columns})
    print('COMPLETE ' + str(out), flush=True)


if __name__ == '__main__':
    main()
