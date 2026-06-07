#!/bin/bash
# Iterate: conflicts → parser → apply → go.  Stops when:
#  - inst says "Pre-installation check" / "Installing files" / "Insert"
#  - or 10 iterations without progress
#  - or skipped-package count converges
set -u
cd "$(dirname "$0")/.."
IC=./target/release/iris-ci
LOG=irix-install-console.log
PARSER=tools/inst-resolve.py

for round in $(seq 1 15); do
  echo "=== round $round ==="
  # Step 1: list conflicts
  $IC serial-send "conflicts" >/dev/null
  sleep 8

  # Step 2: extract latest conflict block.  Use the LAST "Resolve conflicts" marker
  # as the end, and the LAST "Inst> conflicts" or "Inst> go" before it as start.
  res_n=$(grep -n "Resolve conflicts by typing" "$LOG" | tail -1 | cut -d: -f1)
  if [ -z "$res_n" ]; then
    echo "  (no Resolve marker — checking install state)"
    tail -8 "$LOG"
    break
  fi
  # Start: most recent "Inst> conflicts" or "Inst> go" or "ERROR: Conflicts" BEFORE res_n
  start_n=$(awk -v end="$res_n" 'NR<end && (/^Inst> conflicts$/ || /^Inst> go$/ || /ERROR: Conflicts must be resolved/){last=NR} END{print last+0}' "$LOG")
  if [ -z "$start_n" ] || [ "$start_n" = "0" ]; then
    echo "  (no start marker found before $res_n)"; break
  fi
  sed -n "${start_n},${res_n}p" "$LOG" > /tmp/cf.$round.txt
  n_conflicts=$(grep -cE "^\s*[0-9]+a\." /tmp/cf.$round.txt)
  echo "  $n_conflicts conflicts"
  [ "$n_conflicts" = "0" ] && break

  # Step 3: parse + classify
  python3 "$PARSER" < /tmp/cf.$round.txt > /tmp/cm.$round.txt 2> /tmp/sm.$round.txt
  head -1 /tmp/sm.$round.txt | sed 's/^/  /'
  n_skipped=$(grep -c "^#  " /tmp/sm.$round.txt || true)

  # Step 4: apply
  while IFS= read -r line; do
    $IC serial-send "$line" >/dev/null
    sleep 2
  done < /tmp/cm.$round.txt

  # Step 5: try go
  $IC serial-send "go" >/dev/null
  sleep 10
  # Step 6: detect install start
  out_tail=$($IC serial-read 2>&1 | tail -20)
  if echo "$out_tail" | grep -qE "Pre-installation check|Installing/removing|Tarred files|Checking space|Please insert.*CD\.|Removing old|Installing new"; then
    echo "*** INSTALL STARTED ***"
    echo "$out_tail" | tail -5
    exit 0
  fi
done
echo "did not start within 15 rounds; current tail:"
tail -15 "$LOG"
