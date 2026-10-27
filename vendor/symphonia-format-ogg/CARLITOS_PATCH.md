# Carlitos patch to symphonia-format-ogg 0.5.5

Source: the published `symphonia-format-ogg` 0.5.5 crate, under MPL-2.0.
Upstream: https://github.com/pdeljanov/Symphonia/tree/v0.5.5/symphonia-format-ogg

`src/mappings/opus.rs`: read all six bits of an Opus code-3 packet's frame
count. The upstream five-bit mask turns 48 short frames into 16 frames,
miscalculating Ogg timestamps, duration and seek positions for valid 120 ms
packets containing 2.5 ms frames.

Regression: `opus::tests::forty_eight_short_frames_keep_duration_samples_and_seek`
decodes the synthetic 240 ms `import-manyframes.opus` fixture, seeks to its
midpoint and rewinds. The unpatched parser reports only 160 ms; FFmpeg decodes
all 11,520 mono samples. The fixture has two packets of 48 frames followed by
one final frame, with the encoder's 120-sample pre-skip preserved.

The source files retain the upstream copyright and MPL notices. `LICENSE` is
the license text from the upstream v0.5.5 tag. Other source files are unchanged.
