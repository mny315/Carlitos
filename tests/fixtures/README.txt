Synthetic audio for native import/playback tests (mostly one-second 440 Hz tones).
No audiobook recordings are included.

Generated with FFmpeg:
ffmpeg -f lavfi -i sine=frequency=440:duration=1 -ac 1 -ar 44100 -b:a 32k -map_metadata -1 import-mono.mp3
ffmpeg -f lavfi -i sine=frequency=440:duration=1 -ac 2 -ar 32000 -b:a 64k -map_metadata -1 import-stereo.mp3
ffmpeg -f lavfi -i sine=frequency=440:duration=1 -ac 1 -ar 44100 -c:a aac -b:a 32k -map_metadata -1 import.aac
ffmpeg -f lavfi -i sine=frequency=440:duration=1 -ac 2 -ar 32000 -c:a aac -b:a 64k -map_metadata -1 -f mp4 import.m4b
ffmpeg -f lavfi -i sine=frequency=440:duration=1 -ac 2 -ar 48000 -c:a flac -map_metadata -1 import.flac

Tests add their own metadata, chapters and cover images to temporary copies.

HE-AAC fixtures use GStreamer's fdkaacenc, then lossless remuxing with FFmpeg:
ffmpeg -f lavfi -i sine=frequency=440:duration=1 -ac 2 -ar 48000 source.wav
for profile in he-aac-v1 he-aac-v2; do
  gst-launch-1.0 -q filesrc location=source.wav ! wavparse ! audioconvert ! fdkaacenc bitrate=32000 ! "audio/mpeg,mpegversion=4,stream-format=adts,profile=$profile" ! filesink location="$profile.aac"
  ffmpeg -i "$profile.aac" -c:a copy "import-$profile.m4b"
done

import-large-frame.flac.gz covers Android extractors exposing FLAC as decoded PCM.
It contains two seconds of 48 kHz, six-channel, 16-bit synthetic audio in 32768-frame
blocks (393216 decoded bytes per full block). gzip keeps the verbatim fixture small;
tests/android/playback-fixtures.py expands it before packaging the test assets.
Validated with the reference FLAC decoder: flac --test import-large-frame.flac

Generate its source and encode without prediction:
python3 - <<'PY'
import random, struct, wave
rng = random.Random(315)
cycle = struct.pack('<32h', *[rng.randrange(-32768, 32768) for _ in range(32)])
with wave.open('large-frame.wav', 'wb') as output:
    output.setparams((6, 2, 48000, 0, 'NONE', 'not compressed'))
    output.writeframes(cycle * (48000 * 2 * 6 // 32))
PY
ffmpeg -i large-frame.wav -c:a flac -frame_size 32768 -lpc_type none -compression_level 0 import-large-frame.flac
gzip -n -9 import-large-frame.flac
