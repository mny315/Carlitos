Carlitos source code is licensed under MIT (see LICENSE).

The interface uses Slint 1.18.1 by SixtyFPS GmbH, under the Slint Royalty-free
Desktop, Mobile, and Web Applications License 2.0. Its complete license is
included in SLINT-LICENSE.md. The public repository README displays the
official Slint attribution badge under section 2(b) of that license.
The About screen also links to Slint.

On Linux, audio is provided by a pinned, reduced GStreamer build. The packaged plugins
cover MP3 (mpg123), FLAC, WAV and AAC/M4A/M4B (FDK-AAC), image metadata,
tempo adjustment, PulseAudio and ALSA. GStreamer is LGPL-2.1-or-later;
FDK-AAC retains its Fraunhofer license, including its source-distribution and
patent provisions. These libraries are dynamically linked and retain their
own licenses. The portable release includes the libraries rather than requiring
system GStreamer packages. Its runtime-manifest.json records every bundled ELF
file and its build-store origin; flake.lock pins the Nixpkgs source recipes.
SQLite is distributed under its public-domain dedication. Other
Rust dependencies retain the licenses declared in their published packages;
the exact dependency versions are recorded in Cargo.lock.

The shared M4B chapter reader uses mp4ameta 0.13.0 (MIT OR Apache-2.0).
Its source, license texts and local chapter-table patch are in vendor/mp4ameta.

On Windows, decoding and audio output use the operating system's Media Foundation
and XAudio2. GStreamer, GLib, D-Bus, PulseAudio and ALSA are not included.
Sonic by Bill Cox is compiled into the Windows executable under Apache-2.0.
The sonic.c and sonic.h in vendor/sonic come from sonic-rs-sys 0.1.9
(https://crates.io/crates/sonic-rs-sys/0.1.9); the full license is in vendor/sonic/LICENSE.
Local patches preserve input/output buffers when allocation fails and propagate
tempo-processing failures to the caller. They also account for flush padding at
the requested tempo and prevent zero-length pitch steps caused by rounding.
Lofty and mp4ameta read tags and chapters; rfd uses the native Windows file dialog.
The Windows executable embeds DejaVu Sans 2.37 as a fallback for missing system
fonts. The unchanged font is from https://dejavu-fonts.github.io/; its copyright
and license notices are in DEJAVU-LICENSE.txt.
The Windows executable embeds these notices and the Carlitos, Slint, Sonic and
DejaVu licenses. `Carlitos.exe --export-licenses licenses.txt` saves them as a text file.

The interface SVG controls in ui/icons are original Carlitos artwork, licensed
under the same MIT license as Carlitos. Book covers belong to their respective
owners and are read only from the user's local collection.
