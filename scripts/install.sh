#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
prefix="${1:-$HOME/.local}"
if [[ ! -x result/bin/Carlitos ]]; then bash scripts/package.sh; fi
package="$(readlink -f result)"
mkdir -p "$prefix/bin" "$prefix/share/applications" "$prefix/share/icons/hicolor/scalable/apps" "$prefix/share/icons/hicolor/symbolic/apps" "$prefix/share/carlitos"
if [[ -e "$prefix/bin/Carlitos" && ! -L "$prefix/bin/Carlitos" ]]; then
  mv "$prefix/bin/Carlitos" "$prefix/bin/Carlitos.backup-$(date +%s)"
fi
nix-store --add-root "$prefix/share/carlitos/package" --indirect -r "$package" >/dev/null
ln -sfn "$package/bin/Carlitos" "$prefix/bin/Carlitos"
# The desktop session need not have the chosen prefix in PATH. Point its
# launcher at the same immutable package retained by the installation's GC root.
sed "s|^Exec=Carlitos$|Exec=$package/bin/Carlitos|" \
  "$package/share/applications/io.github.mny315.Carlitos.desktop" \
  > "$prefix/share/applications/io.github.mny315.Carlitos.desktop"
chmod 644 "$prefix/share/applications/io.github.mny315.Carlitos.desktop"
install -m644 "$package/share/icons/hicolor/scalable/apps/io.github.mny315.Carlitos.svg" "$prefix/share/icons/hicolor/scalable/apps/"
install -m644 "$package/share/icons/hicolor/symbolic/apps/io.github.mny315.Carlitos-symbolic.svg" "$prefix/share/icons/hicolor/symbolic/apps/"
echo "Installed Carlitos to $prefix/bin/Carlitos"
