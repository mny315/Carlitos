#!/usr/bin/env python3
"""Deterministic source-clock fixtures: tone and 1 s speech / 3 s silence."""
import array
import gzip
import math
import pathlib
import shutil
import sys
import wave

root = pathlib.Path('target/tests/android/playback/fixtures/Carlitos-playback')
root.mkdir(parents=True, exist_ok=True)
for name in ('tone', 'silence'):
    samples = array.array('h', (int(2500 * math.sin(2 * math.pi * 440 * i / 48000))
        if name == 'tone' or i // 48000 % 4 == 0 else 0 for i in range(48000 * 60)))
    if sys.byteorder != 'little':
        samples.byteswap()
    with wave.open(str(root / (name + '.wav')), 'wb') as output:
        output.setparams((1, 2, 48000, 0, 'NONE', 'not compressed'))
        output.writeframes(samples.tobytes())

# Exercise output resampling and compressed decoders as well as native 48 kHz.
for rate, channels in ((16000, 1), (44100, 2), (96000, 2)):
    samples = array.array('h', (int(2500 * math.sin(2 * math.pi * 440 * i / rate))
        for i in range(rate * 8) for _ in range(channels)))
    if sys.byteorder != 'little':
        samples.byteswap()
    with wave.open(str(root / f'pcm-{rate}.wav'), 'wb') as output:
        output.setparams((channels, 2, rate, 0, 'NONE', 'not compressed'))
        output.writeframes(samples.tobytes())
for fixture in pathlib.Path('tests/fixtures').glob('import*'):
    if fixture.suffix == '.gz':
        (root / fixture.stem).write_bytes(gzip.decompress(fixture.read_bytes()))
    else:
        shutil.copyfile(fixture, root / fixture.name)
