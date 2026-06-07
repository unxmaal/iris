# Manual test runbook

Copy-paste each block in order. The whole sequence runs ~5–10 minutes including
IRIX boot. Uses the `iris-ci` wrapper, not raw `nc` — every command is one short
line, no JSON escaping.

## Setup

```bash
cd ~/projects/github/unxmaal/iris

# Build (produces both `iris` and `iris-ci`)
cargo build --release --features lightning

# In iris.toml, uncomment the [scsi.2] scratch block:
#   path = "scratch.raw"  cdrom = false  overlay = false
#   scratch = true        size_mb = 64

# Clean state from any prior run
rm -f /tmp/iris.sock /tmp/iris-ci-*-scsi*.overlay scratch.raw 2>/dev/null
rm -rf saves/.cas saves/test-* 2>/dev/null

# Put iris-ci on PATH so the rest is shorter
alias ci=./target/release/iris-ci
```

## Boot iris and IRIX

```bash
# Launch iris in the background (one terminal, --ci enables the control socket)
./target/release/iris --ci > /tmp/iris.log 2>&1 &
until [ -S /tmp/iris.sock ]; do sleep 1; done

# Boot to root shell — one command replaces the 6-step PROM-menu dance
ci boot          # ~40s on M2 interp
ci login         # ~2s; defaults to root with no password
```

**Expected:** `boot: ready at login` followed by `login: shell ready`. Total ~42 s.

---

## Test 1 — Bundle install + diff

Snapshot a clean baseline, inject a "bundle" via the scratch volume, install it
in IRIX, snapshot the result, see exactly what changed. The `put` command
handles the IRIX `dd bs=512` quirk transparently — you never type a sector count.

```bash
ci save test-1/before

# Build a small "bundle" on the host
echo "fake bundle, marker=$(date +%s)" > /tmp/bundle.txt
tar -cf /tmp/bundle.tar -C /tmp bundle.txt

# Inject it into the guest. iris-ci handles bs=512 and truncation.
ci put /tmp/bundle.tar --to /tmp/bundle.tar

# Extract in the guest
ci run 'cd /tmp && tar xf bundle.tar'
ci run 'cat /tmp/bundle.txt'

# Snapshot post-install
ci save test-1/after

# Diff
ci diff test-1/before test-1/after
du -sh saves/.cas
```

**Expected:**
- `cat /tmp/bundle.txt` echoes the `marker=` line back from the guest.
- `diff` shows small `bank0/bank1` chunk deltas (a few %), banks 2/3 unchanged,
  cow_diff lists new dirty sectors on scsi 1 from the tar extract, devices
  changed includes `mc`, `cpu`, `scsi`.
- `du -sh saves/.cas` ≈ 250–260 MB (one snapshot's worth; second snapshot
  added almost nothing thanks to CAS dedup).

---

## Test 2 — Rollback inner loop

The CI rollback inner loop: save → run test → rollback → repeat.

```bash
ci save test-2/clean
ci restore test-2/clean   # arms the in-memory checkpoint

for run in 1 2 3 4 5; do
  echo "=== run $run ==="
  ci run "echo run-$run > /tmp/run.txt && ls /tmp/run.txt"
  T=$(date +%s%N)
  ci rollback >/dev/null
  T2=$(date +%s%N); echo "rollback: $(( (T2-T)/1000000 )) ms"
  ci run 'ls /tmp/run.txt 2>&1 || echo missing'
done
```

**Expected:**
- Each `rollback` prints in the **40–80 ms range** — in-memory, not disk.
- After every rollback, `ls /tmp/run.txt` says missing (or "No such file") —
  RAM and the SCSI overlay both reverted.

---

## Test 3 — CAS dedup at scale

Take 10 snapshots over a brief idle period and confirm disk usage barely grows.

```bash
for i in 01 02 03 04 05 06 07 08 09 10; do
  ci run "date >> /tmp/log" >/dev/null
  ci save test-3/snap$i >/dev/null
  printf 'snap%s  cas=%s\n' "$i" "$(du -sh saves/.cas | cut -f1)"
done

# Delete every other one and gc
for i in 03 05 07 09; do
  ci delete test-3/snap$i >/dev/null
done
ci gc
du -sh saves/.cas
```

**Expected:**
- `snap01` ≈ 250 MB. Each subsequent snap adds **<5 MB** (idle guest).
- `gc` reports `removed_chunks > 0` and `bytes_freed > 0`.

---

## Test 4 — Determinism check

After save, two cold runs of the same instructions should reach identical state.

```bash
ci save test-4/repeatable
ci validate test-4/repeatable -n 0           # just load → digest
ci validate test-4/repeatable -n 1000000     # run 1M instructions twice + diff
```

**Expected:** Both runs print `deterministic for N instructions (PC=0x...)`. The
1M run completes in ~250–300 ms.

---

## Test 5 — Snapshot tree

`tree` shows parent-chain hierarchy.

```bash
ci save test-5/base
ci restore test-5/base       # restoring stamps `parent` on future saves

ci run 'echo bundle-A >> /tmp/log'
ci save test-5/grep-A

ci restore test-5/base
ci run 'echo bundle-B >> /tmp/log'
ci save test-5/grep-B

ci tree
```

**Expected:** the tree shows `test-5/base` at top with `grep-A` and `grep-B`
indented under it.

---

## Test 6 — Script mode

Replace the test sequence above with a one-line invocation against a `.iris`
file.

```bash
cat > /tmp/scenario.iris <<'EOF'
# scratch volume + bundle install scenario
ping
save test-6/before
put /tmp/bundle.tar --to /tmp/bundle.tar
run "cd /tmp && tar xf bundle.tar"
run "cat /tmp/bundle.txt"
save test-6/after
diff test-6/before test-6/after
EOF

ci script /tmp/scenario.iris
```

**Expected:** each step prefixed with `[ok    Nms]`, plus the natural output
of each command (diff table, etc.). Aborts on first failure.

---

## Test 7 — HTTP registry pull

Ship a snapshot between two "machines" (same machine, different `saves/`).

```bash
# Move our latest snapshot into a registry directory
mkdir -p /tmp/iris-reg/snapshots /tmp/iris-reg/cas
cp -r saves/test-1/after /tmp/iris-reg/snapshots/test-1-after
cp -r saves/.cas/* /tmp/iris-reg/cas/

# Serve it
( cd /tmp/iris-reg && python3 -m http.server 8765 ) &
SVR=$!
sleep 1

# Delete local + pull
rm -rf saves/test-pulled saves/.cas
ci pull http://127.0.0.1:8765 test-pulled
ci pull http://127.0.0.1:8765 test-pulled    # second pull, expect 0 chunks

ci restore test-pulled
ci run 'cat /tmp/bundle.txt'

# Cleanup
kill $SVR
rm -rf /tmp/iris-reg
```

**Expected:**
- First pull fetches all chunks (~270 MB).
- Second pull skips all chunks, transfers only ~3.5 MB of metadata, completes
  in ~20 ms.
- Restore + cat shows the bundle marker — full round-trip working.

---

## Cleanup

```bash
ci quit
sleep 1
rm -f /tmp/iris.sock /tmp/iris-ci-*-scsi*.overlay /tmp/iris.log /tmp/bundle.* scratch.raw
rm -rf saves/.cas saves/test-* saves/test-pulled

# Optionally re-comment [scsi.2] in iris.toml
```

---

## What each test really proves

| Test | Validates |
|---|---|
| Setup / boot | iris-ci wrapper + boot/login macros |
| 1 | Scratch volume + put + diff (Phases 2.4, 3.2) |
| 2 | In-memory rollback + COW overlay revert (Phase 2.1) |
| 3 | CAS dedup (Phase 3.1) + gc (Phase 3.2) |
| 4 | Snapshot determinism (Phase 3.3) — guards every future device change |
| 5 | Parent-chain tracking (Phase 1.2) + tree (Phase 3.2) |
| 6 | Script mode — replaces hand-managed multi-step sequences |
| 7 | HTTP registry pull (Phase 3.4) — Docker-layer-style snapshot sharing |
