Synthetic audio for native import/playback tests (mostly one-second 440 Hz tones).
No audiobook recordings are included.

Generated with FFmpeg:
ffmpeg -f lavfi -i sine=frequency=440:duration=1 -ac 1 -ar 44100 -b:a 32k -map_metadata -1 import-mono.mp3
ffmpeg -f lavfi -i sine=frequency=440:duration=1 -ac 2 -ar 32000 -b:a 64k -map_metadata -1 import-stereo.mp3
ffmpeg -f lavfi -i sine=frequency=440:duration=1 -ac 1 -ar 44100 -c:a aac -b:a 32k -map_metadata -1 import.aac
ffmpeg -f lavfi -i sine=frequency=440:duration=1 -ac 2 -ar 32000 -c:a aac -b:a 64k -map_metadata -1 -f mp4 import.m4b
ffmpeg -f lavfi -i sine=frequency=440:duration=1 -ac 2 -ar 48000 -c:a flac -map_metadata -1 import.flac
ffmpeg -f lavfi -i sine=frequency=440:duration=1 -ac 2 -ar 48000 -c:a libopus -b:a 64k -map_metadata -1 import.opus
ffmpeg -f lavfi -i sine=frequency=523:sample_rate=48000:duration=1.2 -ac 2 -c:a libopus -b:a 48k -map_metadata -1 import-stereo.opus
ffmpeg -f lavfi -i sine=frequency=523:sample_rate=48000:duration=0.02 -ac 1 -c:a libopus -b:a 48k -map_metadata -1 import-short.opus

Tests add their own metadata, chapters and cover images to temporary copies.
import-offset.opus is import.opus with all nonzero Ogg granule positions increased
by 48000 and the affected page CRCs recomputed. It tests nonzero stream origins.

import-manyframes.opus uses 48 short frames per 120 ms Opus packet. Its source:
ffmpeg -f lavfi -i sine=frequency=523:sample_rate=48000:duration=0.24 -ac 1 -c:a libopus -application lowdelay -cutoff 4000 -vbr off -b:a 32k -frame_duration 2.5 -map_metadata -1 shortframes.opus
The 97 audio packets were joined in groups of 48 using libopus's repacketizer,
then written to Ogg pages with updated lacing, sequence numbers and CRCs.
The original 120-sample pre-skip and final granule position 11640 are retained.
FFmpeg decodes exactly 11520 mono samples (240 ms); the fixture tests the six-bit
Opus frame count, full decoding and seeking in the Windows decoder.

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
