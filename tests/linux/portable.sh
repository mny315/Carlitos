#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.."
bundle_dir=""
for candidate in target/portable/linux/Carlitos-*-linux; do
  if [[ -d "$candidate" ]]; then bundle_dir="$candidate"; break; fi
done
if [[ -z "$bundle_dir" ]]; then
  echo 'Build the portable release first: ./build.sh linux' >&2
  exit 1
fi
fixture_dir="$(mktemp -d /tmp/carlitos-portable-fixtures.XXXXXXXX)"
trap 'rm -rf -- "$fixture_dir"' EXIT
export XDG_CACHE_HOME="$fixture_dir/cache"
bash tests/support/fixtures.sh "$fixture_dir"
media_test="$(cargo test --locked --test media_integration --no-run --message-format=json | python3 -c '
import json, sys
for line in sys.stdin:
    item = json.loads(line)
    if item.get("reason") == "compiler-artifact" and item.get("executable"):
        if item["target"]["name"] == "media_integration":
            print(item["executable"])
')"
python3 tests/linux/portable.py "$bundle_dir" --media-test "$media_test" --fixtures "$fixture_dir" "$@"
