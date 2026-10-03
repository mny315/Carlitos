#!/usr/bin/env bash
set -euo pipefail
destination="${1:?fixture directory}"
mkdir -p "$destination/Книга с пробелом/Disc 1" "$destination/Вторая книга"
ffmpeg -hide_banner -loglevel error -y -f lavfi -i 'sine=frequency=440:duration=12' -ac 2 -ar 48000 "$destination/sample.wav"
for extension in mp3 flac aac m4a m4b; do
  ffmpeg -hide_banner -loglevel error -y -i "$destination/sample.wav" -metadata title="Тест $extension" -metadata artist="Carlitos" -metadata album="Проверка" "$destination/sample.$extension"
done
ffmpeg -hide_banner -loglevel error -y -f lavfi -i 'color=c=blue:s=64x64' \
  -frames:v 1 -update 1 "$destination/cover.png"
for extension in mp3 flac m4a; do
  ffmpeg -hide_banner -loglevel error -y -i "$destination/sample.$extension" \
    -i "$destination/cover.png" -map 0:a -map 1:v -c copy \
    -disposition:v attached_pic "$destination/covered.$extension"
done
# Exercise the AAC profiles used by low-bitrate books as well as AAC-LC.
for profile in he-aac-v1 he-aac-v2; do
  gst-launch-1.0 -q filesrc location="$destination/sample.wav" ! wavparse ! \
    audioconvert ! fdkaacenc bitrate=32000 ! \
    "audio/mpeg,mpegversion=4,stream-format=adts,profile=$profile" ! \
    filesink location="$destination/$profile.aac"
  ffmpeg -hide_banner -loglevel error -y -i "$destination/$profile.aac" \
    -c:a copy "$destination/$profile.m4b"
done
cat > "$destination/chapters.txt" <<'EOF'
;FFMETADATA1
[CHAPTER]
TIMEBASE=1/1000
START=0
END=5000
title=Opening
[CHAPTER]
TIMEBASE=1/1000
START=5000
END=12000
title=Second chapter
EOF
ffmpeg -hide_banner -loglevel error -y -i "$destination/sample.m4b" -i "$destination/chapters.txt" -map_metadata 1 -map_chapters 1 -c copy "$destination/chapters.m4b"
for part in 1 2 10; do
  cp "$destination/sample.wav" "$destination/Книга с пробелом/Disc 1/$part.wav"
done
cp "$destination/sample.wav" "$destination/Вторая книга/1.wav"
printf 'not audio\n' > "$destination/broken.mp3"
ln -sfn . "$destination/Книга с пробелом/loop"
