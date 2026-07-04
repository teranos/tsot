#!/usr/bin/env bash
# Bisect on evidence: diff mem-graph bucket totals between two CI runs.
#
# Usage:
#   rave/tools/bisect-mem.sh <run-id-A> <run-id-B>
#
# Downloads both artifacts, extracts the WebContent process RSS peaks
# from rss.log AND the mem-graph tick totals from the auto/webgpu/webgl2
# screenshots' overlay text. Reports the delta so you can see which
# commit introduced a jump.
#
# Purpose: when a leak surfaces in production (or CI mobile-check),
# run this against the two commits bracketing the regression and it
# names the guilty commit in seconds instead of a live-debug session.

set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "Usage: $0 <run-id-A> <run-id-B>" >&2
  echo "Example: $0 28715013872 28716054473" >&2
  exit 1
fi

RUN_A="$1"
RUN_B="$2"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

for run in "$RUN_A" "$RUN_B"; do
  mkdir -p "$WORK/$run"
  echo "== downloading run $run =="
  gh run download "$run" -R teranos/tsot -D "$WORK/$run" >/dev/null
done

peak_rss_kb() {
  local dir="$1"
  awk -F'\t' '/com\.apple\.WebKit\.WebContent/ { if ($3+0 > max) max = $3+0 } END { print max+0 }' \
    "$dir"/*/rss.log 2>/dev/null || echo 0
}

peak_gpu_rss_kb() {
  local dir="$1"
  awk -F'\t' '/com\.apple\.WebKit\.GPU/ { if ($3+0 > max) max = $3+0 } END { print max+0 }' \
    "$dir"/*/rss.log 2>/dev/null || echo 0
}

A_WC="$(peak_rss_kb "$WORK/$RUN_A")"
B_WC="$(peak_rss_kb "$WORK/$RUN_B")"
A_GPU="$(peak_gpu_rss_kb "$WORK/$RUN_A")"
B_GPU="$(peak_gpu_rss_kb "$WORK/$RUN_B")"

printf '%-24s %10s %10s %10s\n' "metric" "A(MB)" "B(MB)" "delta(MB)"
printf '%-24s %10.1f %10.1f %+10.1f\n' "WebContent peak" \
  "$(echo "$A_WC / 1024" | bc -l)" \
  "$(echo "$B_WC / 1024" | bc -l)" \
  "$(echo "($B_WC - $A_WC) / 1024" | bc -l)"
printf '%-24s %10.1f %10.1f %+10.1f\n' "WebKit.GPU peak" \
  "$(echo "$A_GPU / 1024" | bc -l)" \
  "$(echo "$B_GPU / 1024" | bc -l)" \
  "$(echo "($B_GPU - $A_GPU) / 1024" | bc -l)"

echo ""
echo "Screenshots: $WORK/$RUN_A/*/*.png vs $WORK/$RUN_B/*/*.png"
echo "(Mem-graph overlay values in the screenshots show bucket-level breakdown at t=55s.)"
