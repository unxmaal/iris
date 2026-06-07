#!/bin/sh
# Build iris_bench on Linux (system openssl 3.x + zlib).
set -e
cd "$(dirname "$0")"
cc -std=c99 -O2 -Wno-deprecated-declarations iris_bench.c -lcrypto -lz -o iris_bench
echo "built: $(pwd)/iris_bench"
