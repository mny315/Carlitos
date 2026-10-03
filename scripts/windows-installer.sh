#!/usr/bin/env bash
# Called inside the Windows Nix shell and xvfb-run by windows.sh.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
executable="$1"
assets="$2"
version="$3"
export WINEPREFIX="$PWD/target/windows-tools/wine"
export WINEARCH=win64 WINEDEBUG=-all WINEDLLOVERRIDES='mscoree,mshtml=d'
staging="$(mktemp -d "$PWD/target/windows-tools/installer.XXXXXXXX")"
# Stop the private Wine server before xvfb-run tears down its display.
trap 'wineserver -k; wineserver -w; rm -rf -- "$staging"' EXIT
compiler="$WINEPREFIX/drive_c/inno-$CARLITOS_INNO_VERSION/ISCC.exe"
if [[ ! -f "$compiler" ]]; then
  echo "Preparing Inno Setup $CARLITOS_INNO_VERSION..."
  wine "$CARLITOS_INNO_SETUP" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- \
    "/DIR=C:\\inno-$CARLITOS_INNO_VERSION"
fi

# Publish only after successful compilation; failed builds keep the last release.
echo "Building Windows installer..."
wine "$compiler" /Qp "/DAppVersion=$version" \
  "/DSourceExe=$(winepath -w "$executable")" \
  "/DAssetsDir=$(winepath -w "$assets")" \
  "/O$(winepath -w "$staging")" \
  "$(winepath -w "$PWD/data/windows/installer.iss")"
(
  cd -- "$staging"
  sha256sum Carlitos.exe > Carlitos.exe.sha256
)
destination="$PWD/target/releases"
mkdir -p "$destination"
mv -- "$staging/Carlitos.exe" "$staging/Carlitos.exe.sha256" "$destination/"
rm -f -- "$destination/setup.exe" "$destination/setup.exe.sha256"
# Remove the former portable artifacts, preserving any library saved beside them.
rm -f -- "$destination/windows-x64/Carlitos.exe" \
  "$destination/windows-x64/Carlitos.exe.sha256" "$destination/windows-x64/Carlitos.exe.ppdb"
if [[ -d "$destination/windows-x64" ]]; then
  rmdir --ignore-fail-on-non-empty -- "$destination/windows-x64"
fi
printf '\nWindows installer: %s/Carlitos.exe\n' "$destination"
