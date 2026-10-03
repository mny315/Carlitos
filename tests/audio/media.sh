#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.."
fixture_dir="$(mktemp -d /tmp/carlitos-fixtures.XXXXXXXX)"
trap 'rm -rf -- "$fixture_dir"' EXIT
export XDG_CACHE_HOME="$fixture_dir/cache"
bash tests/support/fixtures.sh "$fixture_dir"
CARLITOS_FIXTURES="$fixture_dir" cargo test --locked --test media_integration -- --ignored --nocapture --test-threads=1
