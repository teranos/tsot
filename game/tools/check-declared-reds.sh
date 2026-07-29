#!/usr/bin/env bash
# A test declared RED must actually be red.
#
# The gate already refuses a bare `#[ignore]`: every skip has to say why
# it is off. That catches a lie in one direction — a test switched off
# with no reason. It says nothing about the other direction, where a
# test carries `#[ignore = "RED: ..."]` and would pass if it ran.
#
# That is not hypothetical. It happened in this repo: a test asserting a
# cursor on the sky names no destination was labelled RED and passed
# vacuously against the stub, and it took someone noticing by eye. A
# green run reported "3 declared reds" and one of them was not red.
#
# Why it matters more than it sounds: the RED label is what buys the
# strict-TDD gate its one distinction — red because not built yet versus
# red because broken. A RED that passes silently converts a green CI
# into a claim nobody checked, which is exactly what the label exists to
# prevent.
#
# Usage: check-declared-reds.sh <normal-test-output>
#
# Argument is the captured output of an ORDINARY `cargo test` run, where
# ignored tests print their reason. This script then runs the ignored
# set itself and compares.

set -uo pipefail

normal="${1:?usage: check-declared-reds.sh <normal-test-output>}"
if [ ! -f "$normal" ]; then
  echo "::error::no test output at $normal — cannot audit declared reds"
  exit 1
fi

# Ignored tests print their reason only when SKIPPED, so the names of
# the declared reds come from the ordinary run.
reds=$(grep -E '^test [^ ]+ \.\.\. ignored, RED:' "$normal" \
       | sed -E 's/^test ([^ ]+) .*/\1/' | sort -u)

if [ -z "$reds" ]; then
  echo "[declared-reds] none declared — nothing to audit"
  exit 0
fi

# In the --ignored run they actually execute, so the reason is gone and
# the line is a plain ok/FAILED.
ignored_out="$(dirname "$normal")/ignored-output.txt"
cargo +nightly test --no-fail-fast -- --ignored >"$ignored_out" 2>&1
passing=$(grep -E '^test [^ ]+ \.\.\. ok$' "$ignored_out" \
          | sed -E 's/^test ([^ ]+) .*/\1/' | sort -u)

liars=$(comm -12 <(echo "$reds") <(echo "$passing"))

if [ -n "$liars" ]; then
  echo "::error::test declared RED but passing — the label is a lie"
  echo "$liars" | sed 's/^/    /'
  echo
  echo "Either it is not red (drop the #[ignore], it is a guard, not a"
  echo "red) or it is not testing what its name says."
  exit 1
fi

echo "[declared-reds] $(echo "$reds" | wc -l | tr -d ' ') declared, all actually failing"
exit 0
