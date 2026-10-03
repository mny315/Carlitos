#!/usr/bin/env python3
"""Bundle the pinned Linux ELF runtime; no Nix, FUSE or container at launch."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile


def elf_info(path, option):
    return subprocess.check_output(["patchelf", option, str(path)], text=True).strip()


class Bundle:
    def __init__(self, destination, search=()):
        self.root = Path(destination)
        self.search = list(search)
        self.files = {}
        self.origins = {}

    def elf(self, source, relative):
        source = Path(source).resolve(strict=True)
        target = self.root / relative
        previous = self.origins.get(str(relative))
        if previous:
            if previous != source and previous.read_bytes() != source.read_bytes():
                raise RuntimeError(f"Conflicting libraries: {source} and {previous}")
            return target
        self.origins[str(relative)] = source
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        target.chmod(0o755)
        search = [source.parent]
        for directory in elf_info(source, "--print-rpath").split(":"):
            if directory:
                search.append(Path(directory.replace("${ORIGIN}", str(source.parent)).replace("$ORIGIN", str(source.parent))))
        search.extend(self.search)
        for needed in elf_info(source, "--print-needed").splitlines():
            dependency = Path(needed) if needed.startswith("/") else next(
                (directory / needed for directory in search if (directory / needed).is_file()), None
            )
            if dependency is None:
                raise RuntimeError(f"Cannot resolve {needed} required by {source}")
            name = Path(needed).name
            self.elf(dependency, Path("lib") / name)
            if needed != name:
                subprocess.run(["patchelf", "--replace-needed", needed, name, str(target)], check=True)
        # Never search the build machine's /nix/store at runtime.
        relative_lib = os.path.relpath(self.root / "lib", target.parent)
        rpath = "$ORIGIN" if relative_lib == "." else f"$ORIGIN/{relative_lib}"
        if not source.name.startswith("ld-linux"):
            subprocess.run(["patchelf", "--set-rpath", rpath, str(target)], check=True)
        self.files[str(relative)] = {
            "source": str(source),
            "sha256": hashlib.sha256(target.read_bytes()).hexdigest(),
        }
        return target


LAUNCHER = '''#!/bin/sh
set -eu
# Resolve relative paths before changing directory, including paths with spaces.
bundle_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
export GST_PLUGIN_SYSTEM_PATH_1_0="$bundle_dir/lib/gstreamer-1.0"
export GST_PLUGIN_PATH_1_0=
export GST_PLUGIN_SYSTEM_PATH="$GST_PLUGIN_SYSTEM_PATH_1_0"
export GST_PLUGIN_PATH=
export GST_PLUGIN_SCANNER_1_0="$bundle_dir/libexec/gst-plugin-scanner"
export GST_PLUGIN_SCANNER="$GST_PLUGIN_SCANNER_1_0"
export GST_REGISTRY_1_0="${XDG_CACHE_HOME:-$HOME/.cache}/carlitos/gstreamer-@ARCH@-@GST@.bin"
export FONTCONFIG_FILE="$bundle_dir/share/fontconfig/fonts.conf"
export XKB_CONFIG_ROOT="$bundle_dir/share/X11/xkb"
export XLOCALEDIR="$bundle_dir/share/X11/locale"
export ALSA_CONFIG_DIR="$bundle_dir/share/alsa"
export ALSA_CONFIG_PATH="$ALSA_CONFIG_DIR/alsa.conf"
# A software renderer works without bundling a GPU driver from the build host.
# Users can explicitly select winit-femtovg with their system's GL drivers.
export SLINT_BACKEND="${SLINT_BACKEND:-winit-software}"
exec "$bundle_dir/lib/@LOADER@" --inhibit-cache --library-path "$bundle_dir/lib" "$bundle_dir/libexec/Carlitos" "$@"
'''

SCANNER = '''#!/bin/sh
set -eu
bundle_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
exec "$bundle_dir/lib/@LOADER@" --inhibit-cache --library-path "$bundle_dir/lib" "$bundle_dir/libexec/gst-plugin-scanner.bin" "$@"
'''

FONTS = '''<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <dir prefix="relative">../fonts</dir>
  <dir>/usr/share/fonts</dir>
  <dir>/usr/local/share/fonts</dir>
  <dir>/run/current-system/sw/share/X11/fonts</dir>
  <dir prefix="xdg">fonts</dir>
  <cachedir prefix="xdg">fontconfig</cachedir>
  <alias><family>sans-serif</family><prefer><family>DejaVu Sans</family></prefer></alias>
</fontconfig>
'''


def copy_data(source, target):
    # Nix store directories are read-only; keep the assembled tree writable.
    source, target = Path(source), Path(target)
    if source.is_dir():
        target.mkdir(parents=True, exist_ok=True)
        for child in source.iterdir():
            copy_data(child, target / child.name)
    else:
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("spec", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    spec = json.loads(args.spec.read_text())
    archive_root = args.output / f"Carlitos-{spec['version']}-{spec['arch']}-linux"
    archive_root.mkdir(parents=True)
    binary = Path(spec["package"]) / "bin/.Carlitos-wrapped"
    if not binary.is_file():
        raise RuntimeError(f"Missing unwrapped executable: {binary}")
    loader = Path(elf_info(binary, "--print-interpreter"))
    bundle = Bundle(archive_root, [loader.parent])
    # The ELF loader can execute directly; the application and helper use it
    # explicitly rather than an interpreter at a fixed path on the host.
    bundle.elf(binary, "libexec/Carlitos")
    bundle.elf(loader, Path("lib") / loader.name)
    bundle.elf(spec["scanner"], "libexec/gst-plugin-scanner.bin")
    for path in spec["pluginDirs"]:
        for plugin in sorted(Path(path).glob("*.so")):
            bundle.elf(plugin, Path("lib/gstreamer-1.0") / plugin.name)
    for library in spec["libraries"]:
        bundle.elf(library, Path("lib") / Path(library).name)
    for name, template in [("Carlitos", LAUNCHER), ("libexec/gst-plugin-scanner", SCANNER)]:
        text = template.replace("@LOADER@", loader.name).replace("@ARCH@", spec["arch"]).replace("@GST@", spec["gstreamerVersion"])
        path = archive_root / name
        path.write_text(text)
        path.chmod(0o755)
    for item in spec["data"]:
        copy_data(item["source"], archive_root / item["target"])
    fonts = archive_root / "share/fontconfig/fonts.conf"
    fonts.parent.mkdir(parents=True, exist_ok=True)
    fonts.write_text(FONTS)
    (archive_root / "runtime-manifest.json").write_text(json.dumps({
        "version": spec["version"], "arch": spec["arch"],
        "gstreamer": spec["gstreamerVersion"], "files": bundle.files,
        "data": spec["data"],
        "recipes": spec["recipes"],
        "plugins": sorted(p.name for p in (archive_root / "lib/gstreamer-1.0").glob("*.so")),
    }, indent=2) + "\n")
    (archive_root / "README.txt").write_text(
        "Carlitos — portable Linux release\n\n"
        "Run ./Carlitos from this directory. Keep the directory together.\n"
        "No Nix, GStreamer installation, FUSE or root privileges are needed.\n"
        "Requires a Linux desktop with Wayland or X11 and PulseAudio/PipeWire\n"
        "or ALSA. The software renderer is used by default.\n"
        "Supported: MP3, FLAC, WAV, AAC (LC/HE), M4A and M4B.\n"
        "Settings and the library use the normal XDG directories.\n\n"
        "Build provenance: runtime-manifest.json, flake.lock and Cargo.lock.\n"
        "Licenses and source notices: share/licenses/carlitos/.\n"
    )
    # Make a deterministic archive, independent of build timestamps and uid.
    def normalize(info):
        info.uid = info.gid = 0
        info.uname = info.gname = "root"
        info.mtime = 1
        return info
    archive = args.output / f"{archive_root.name}.tar.xz"
    with tarfile.open(archive, "w:xz", preset=6) as tar:
        tar.add(archive_root, arcname=archive_root.name, filter=normalize)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(archive.suffix + ".sha256").write_text(f"{digest}  {archive.name}\n")
    print(f"{len(bundle.files)} ELF files, archive {archive.stat().st_size / 1024**2:.1f} MiB")


if __name__ == "__main__":
    main()
