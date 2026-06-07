#!/bin/sh
# Build iris_bench on IRIX 6.5 with MIPSpro cc (openssl 0.9.7 + zlib).
# Copy this file and iris_bench.c onto the IRIX guest, then run it there.
set -e
cc -c99 -O2 -n32 -mips3 iris_bench.c -lcrypto -lz -o iris_bench
echo "built: ./iris_bench"
