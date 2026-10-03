#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.."
mkdir -p target/tests/sonic
cc -O1 -g -fsanitize=address,undefined -fno-sanitize-recover=all \
  tests/support/sonic.c -lm -o target/tests/sonic/allocation-failures
target/tests/sonic/allocation-failures
