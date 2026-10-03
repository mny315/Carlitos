#!/usr/bin/env python3
"""Export corresponding upstream sources and Carlitos build recipes for a bundle."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile


def nix_json(*arguments):
    return json.loads(subprocess.check_output(["nix", *arguments], text=True))


def valid_deriver(root):
    candidates = subprocess.check_output(
        ["nix-store", "--query", "--valid-derivers", root], text=True).splitlines()
    if not candidates:
        raise RuntimeError(f"No local build recipe for {root}; rebuild the pinned portable package first")
    return candidates[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--output", type=Path, default=Path("target/releases"))
    args = parser.parse_args()
    manifest = json.loads((args.bundle / "runtime-manifest.json").read_text())
    if "recipes" not in manifest:
        raise RuntimeError("Bundle has no pinned build recipes; rebuild it with ./build.sh linux")
    recipes_source = Path(manifest["recipes"])
    origins = list(manifest["files"].values()) + manifest.get("data", [])
    paths = [str(Path(file["source"]).resolve(strict=True)) for file in origins]
    roots = sorted({"/".join(path.split("/")[:4]) for path in paths
                    if len(path.split("/")) > 4})
    # Binary caches can name a deriver that is not present locally. Query the
    # available derivations producing the same output instead.
    derivations = [valid_deriver(root) for root in roots]
    descriptions = nix_json("derivation", "show", *derivations)
    descriptions = descriptions.get("derivations", descriptions)
    sources = {}
    pending = list(descriptions.items())
    while pending:
        drv, info = pending.pop()
        attributes = info.get("structuredAttrs", info.get("env", {}))
        if "__json" in attributes:
            attributes = json.loads(attributes["__json"])
        source = attributes.get("src")
        if not source:
            # Split data packages (e.g. DejaVu's minimal font) copy files from
            # another derivation. Follow those explicit source references.
            references = set(re.findall(r"/nix/store/[0-9a-z]{32}-[^\s/\"';]+",
                                        attributes.get("buildCommand", "")))
            if not references:
                raise RuntimeError(f"No source recorded for {drv}")
            parents = nix_json("derivation", "show", *[valid_deriver(p) for p in references])
            for name, parent in parents.get("derivations", parents).items():
                if name not in descriptions:
                    descriptions[name] = parent
                    pending.append((name, parent))
            continue
        sources.setdefault(source, []).append(Path(drv).name)
        patches = attributes.get("patches", [])
        if isinstance(patches, str):
            patches = patches.split()
        for patch in patches:
            if patch.startswith("/nix/store/"):
                sources.setdefault(patch, []).append(Path(drv).name)
    # Output paths retain their registered derivers, allowing Nix to fetch or
    # realise the exact sources used by the already-built release.
    subprocess.run(["nix", "build", "--no-link", *sources, str(recipes_source)], check=True)
    args.output.mkdir(parents=True, exist_ok=True)
    name = f"Carlitos-{manifest['version']}-{manifest['arch']}-linux-sources"
    with tempfile.TemporaryDirectory(prefix="carlitos-sources-") as temporary:
        root = Path(temporary) / name
        root.mkdir()
        for source in sources:
            path = Path(source)
            target = root / "upstream" / path.name
            target.parent.mkdir(exist_ok=True)
            if path.is_dir():
                shutil.copytree(path, target, symlinks=True)
            else:
                shutil.copyfile(path, target)
        recipes = root / "carlitos"
        # Export the exact snapshot recorded by the bundle, even if this checkout
        # has since changed or this exporter is run outside the project tree.
        shutil.copytree(recipes_source, recipes, symlinks=True,
                        ignore=shutil.ignore_patterns("__pycache__"))
        (root / "sources.json").write_text(json.dumps(sources, indent=2) + "\n")
        (root / "derivations.json").write_text(json.dumps(descriptions, indent=2) + "\n")
        (root / "runtime-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        (root / "README.txt").write_text(
            "Corresponding sources for the bundled native libraries.\n"
            "Each upstream archive/directory contains its license and copyright notices.\n"
            "sources.json maps pinned source paths to the package derivations.\n"
            "Carlitos and its audio build overrides are in carlitos/.\n"
            "flake.lock pins Nixpkgs, including the upstream patches and build recipes.\n"
            "Rust dependency versions and checksums are in Cargo.lock.\n"
            "Keep this source archive available alongside the binary release.\n"
        )
        def normalize(info):
            info.uid = info.gid = 0
            info.uname = info.gname = "root"
            info.mtime = 1
            # Store snapshots are read-only; extracted sources must be editable
            # and allow Cargo/Nix to create their build outputs.
            if info.isfile() or info.isdir():
                info.mode |= 0o200
            return info
        archive = args.output / (name + ".tar.xz")
        with tarfile.open(archive, "w:xz", preset=6) as tar:
            tar.add(root, arcname=name, filter=normalize)
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        archive.with_suffix(".xz.sha256").write_text(f"{digest}  {archive.name}\n")
        print(f"{archive}: {len(sources)} source trees/archives, {archive.stat().st_size / 1024**2:.1f} MiB")


if __name__ == "__main__":
    main()
