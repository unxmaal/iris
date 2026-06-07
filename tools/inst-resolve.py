#!/usr/bin/env python3
"""
Parse the output of inst's `conflicts` command and pick a resolution for each
numbered conflict.

For each conflict, options are listed as `Na.`, `Nb.`, `Nc.`, etc.

Strategy:
  - 'a' (Do not install X)              → SKIP (X gets removed from selection)
  - any other option (b, c, …)
      - if its text contains "from an additional distribution" or
        "insert another CD" → unavailable, skip
      - else → satisfiable (prereqs are on a loaded CD)

For each conflict, prefer the *highest* satisfiable option (so 'c' wins over
'b' wins over 'a'). 'a' is the fallback when no other option is satisfiable —
it means the package gets dropped because we don't have the media.

Outputs:
  - to stdout: a single `conflicts N1c N2b N3a ...` string
  - to stderr: human-readable summary of what got skipped and why

Usage: cat conflicts.txt | inst-resolve.py
"""
import re
import sys

text = sys.stdin.read()

# Find each option line: e.g. " 12a. Do not install ..."  or " 12b. Also install ..."
# An option spans from "  Na." to the next "  Mx." or to an unindented line.
# Simpler approach: split on the option-letter prefix, keep the (n, letter, body) triples.

opt_re = re.compile(r"^\s*(\d+)([a-z])\.\s+(.+?)(?=^\s*\d+[a-z]\.|^Resolve conflicts|^Inst>|\Z)",
                    re.MULTILINE | re.DOTALL)

options = {}                # {conflict_num: [(letter, body), ...]}
for m in opt_re.finditer(text):
    n, letter, body = int(m.group(1)), m.group(2), m.group(3).strip()
    options.setdefault(n, []).append((letter, body))

if not options:
    print("# (no conflicts found in input)", file=sys.stderr)
    sys.exit(0)

unavailable_marker = re.compile(r"from an additional distribution|insert another CD|Open new distribution", re.I)

choices = []                # ["1c", "2b", ...]
skipped_pkgs = []           # [(conflict_num, pkg_name, reason)]
satisfied_pkgs = []         # [(conflict_num, letter, pkg_name)]

# Determine conflict header text for each numbered conflict by looking at the
# lines just before its 'a' option.  The header tells us whether it's an
# "is incompatible with" conflict (prefer 'a' = skip the old) or a
# "missing prerequisites" conflict (prefer 'b'/'c' = install the prereq).
header_re = re.compile(r"(.+?)\s+(?:cannot be installed because of missing prerequisites|is incompatible with|are required and must be installed|incompatible with your hardware)",
                       re.DOTALL)
def conflict_kind(num: int) -> str:
    """Return 'incompat', 'missing_prereq', 'required', 'hardware', or 'unknown'.

    The header for conflict N is the most recent non-option, non-blank line
    above its `Na.` line.  Earlier conflicts' option text must not be picked
    up — that's the bug we hit when the previous conflict's headline still
    sat in a 6-line window.
    """
    a_opt_pat = re.compile(rf"^\s*{num}a\.", re.MULTILINE)
    m = a_opt_pat.search(text)
    if not m:
        return "unknown"
    # Walk back line-by-line, skipping blank lines and any line that starts
    # with whitespace followed by a digit (i.e. an option line of an earlier
    # conflict).  The first non-option, non-blank line is the header (possibly
    # wrapped onto multiple physical lines — we glue back).
    prefix = text[:m.start()]
    lines = prefix.split("\n")
    header_parts = []
    for ln in reversed(lines):
        stripped = ln.strip()
        if not stripped:
            if header_parts:
                break
            continue
        if re.match(r"^\s+\d+[a-z]\.", ln):
            # option line of a previous conflict; ignore
            if header_parts:
                break
            continue
        header_parts.insert(0, stripped)
        if re.search(r"(cannot be installed because|is incompatible with|are required and must be installed|incompatible with your hardware)", " ".join(header_parts)):
            break
    header = " ".join(header_parts)
    if "incompatible with your hardware" in header:
        return "hardware"
    if "is incompatible with" in header:
        return "incompat"
    if "are required and must be installed" in header:
        return "required"
    if "cannot be installed because of missing prerequisites" in header:
        return "missing_prereq"
    return "unknown"

# First pass: collect packages that have an "incompat" or "hardware" conflict.
# For those packages, ALL conflicts (including missing-prereq) must resolve as
# "don't install" — otherwise inst loops between trying to install (to satisfy
# the prereq) and being told it's incompatible.
pkgs_to_skip = set()
for n in sorted(options):
    pkg_match = re.search(r"Do not install (\S+)", options[n][0][1])
    pkg = pkg_match.group(1) if pkg_match else None
    if pkg and conflict_kind(n) in ("incompat", "hardware"):
        pkgs_to_skip.add(pkg)

for n in sorted(options):
    opts = options[n]
    # Find which package this conflict is about — appears at start of the 'a' option's body.
    pkg_match = re.search(r"Do not install (\S+)", opts[0][1])
    pkg_name = pkg_match.group(1) if pkg_match else "?"
    kind = conflict_kind(n)

    # If this package is on the skip-list because of an incompat/hardware
    # conflict elsewhere, force 'a' (don't install) on every conflict for it.
    if pkg_name in pkgs_to_skip:
        choices.append(f"{n}a")
        satisfied_pkgs.append((n, 'a', pkg_name))
        continue

    chosen = None
    if kind in ("incompat", "hardware", "required"):
        # For "X is incompatible with Y" → 'a' = don't install X (which is the OLD one
        # being incompatible with our newer kept Y).  Keep Y.
        # For "Z is required and must be installed" → 'a' = install Z (despite the wording).
        # For hardware-incompat → 'a' = don't install (there's no fixing it).
        chosen = 'a'
    else:
        # Missing-prereq conflicts: pick the highest-letter option whose prereqs
        # are on a loaded CD AND don't reference packages we've already decided
        # to skip (because they had an incompat conflict).  If 'b' says
        # "Also install ViewKit_eoe.sw.base (..)" but ViewKit_eoe.sw.base is on
        # the skip-list because of its own incompat with eoe.sw.base, choosing
        # 'b' would loop: inst tries to install it, sees the incompat, fails.
        for letter, body in reversed(opts):
            if letter == 'a':
                continue
            if unavailable_marker.search(body):
                continue
            # Does the body name a package we already decided to skip?
            depends_on_skipped = False
            for skip_pkg in pkgs_to_skip:
                # match the package name as a whole-word
                if re.search(r"(?<![\w.])" + re.escape(skip_pkg) + r"(?![\w.])", body):
                    depends_on_skipped = True
                    break
            if depends_on_skipped:
                continue
            chosen = letter
            break
        if chosen is None:
            chosen = 'a'
            skipped_pkgs.append((n, pkg_name, "no non-'a' option satisfiable (missing media or depends on a skipped package)"))
            choices.append(f"{n}a")
            continue

    choices.append(f"{n}{chosen}")
    satisfied_pkgs.append((n, chosen, pkg_name))

# Output to stderr: summary
print(f"# {len(choices)} conflicts → {len(satisfied_pkgs)} satisfied, {len(skipped_pkgs)} skipped (missing media).",
      file=sys.stderr)
if skipped_pkgs:
    print(f"# Skipped (missing media):", file=sys.stderr)
    for n, pkg, why in skipped_pkgs:
        print(f"#   {n:>3}. {pkg}  — {why}", file=sys.stderr)

# Output ONE single "conflicts" command containing every choice.
# Multiple batches don't work — resolving the first batch renumbers the rest,
# so batch 2's "16a 17a ..." would refer to conflicts that no longer exist.
print("conflicts " + " ".join(choices))
