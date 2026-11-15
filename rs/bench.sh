#!/bin/bash
# Median wall time of the caelestia binary for the hot commands, with warm caches. Runs inside the sandbox.
# Usage: rs/bench.sh [runs] [binary]   (default 21 runs, rs/target/release/caelestia)
set -euo pipefail
source "$(dirname "$0")/sandbox.sh"
RUNS=${1:-21}
RS=${2:-$RS_BIN}
ensure_template
reset_sandbox

W=$HOME/Pictures/Wallpapers
cmds=(
    "scheme get"
    "scheme list"
    "scheme set --notify -m dark"
    "wallpaper -p $W/wallhaven-rq6qp7.jpg"
    "wallpaper -f $W/wallhaven-rq6qp7.jpg"
    "wallpaper -f $W/Live/singularity-cosmic-vortex-moewalls-com.mp4"
    "wallpaper -r $SB/walls/pair -n"
)

# Timer runs inside the namespace, so the spawn floor is measured the same way.
# It prints the median of RUNS runs in ms, after 2 untimed warm-up runs.
timer='
import subprocess, sys, time, statistics
runs, cmd = int(sys.argv[1]), sys.argv[2:]
ts = []
for i in range(runs + 2):
    t = time.perf_counter_ns()
    subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if i >= 2:
        ts.append((time.perf_counter_ns() - t) / 1e6)
print(f"{statistics.median(ts):.2f}")
'
median() { sandbox_run python3 -I -c "$timer" "$RUNS" "$@"; }

printf '%-62s %8s\n' command ms
printf '%-62s %8s\n' "(spawn floor: /usr/bin/true)" "$(median /usr/bin/true)"
for c in "${cmds[@]}"; do
    # shellcheck disable=SC2086
    printf '%-62s %8s\n' "${c/$W\//}" "$(median "$RS" $c)"
done
if [[ -s $FALLBACK_MARK ]]; then
    echo ":: warning: some runs called Python ($(wc -l < "$FALLBACK_MARK") calls), so a cache was cold" >&2
fi
