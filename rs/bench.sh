#!/bin/bash
# Median wall time, Python CLI vs the Rust CLI, for the hot commands. Runs inside the parity sandbox.
# Usage: rs/bench.sh [runs] [binary]   (default 21 runs, rs/target/release/caelestia)
# A missing Rust binary benchmarks Python only (the pre-port baseline).
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

# Timer runs inside the namespace so the spawn cost is the same for both CLIs.
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

printf '%-62s %10s %10s %8s\n' command "python ms" "rust ms" speedup
printf '%-62s %10s\n' "(spawn floor: /usr/bin/true)" "$(median /usr/bin/true)"
for c in "${cmds[@]}"; do
    # shellcheck disable=SC2086
    py=$(median "$PY_BIN" $c)
    rs=-
    sp=-
    if [[ -x $RS ]]; then
        # shellcheck disable=SC2086
        rs=$(median "$RS" $c)
        sp=$(awk -v a="$py" -v b="$rs" 'BEGIN { printf "%.1fx", a / b }')
    fi
    printf '%-62s %10s %10s %8s\n' "${c/$W\//}" "$py" "$rs" "$sp"
done
if [[ -s $FALLBACK_MARK ]]; then
    echo ":: warning: some Rust runs fell back to Python ($(wc -l < "$FALLBACK_MARK") calls)" >&2
fi
