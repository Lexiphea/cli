#!/bin/bash
# A/B parity: run the Python CLI and the Rust CLI on identical fresh sandboxes (same path) and compare
# stdout, exit code, every file in the sandbox (type, mode, link target, sha256, fake /dev/pts
# contents) and the sorted shim command log. Also checks whether Rust ran natively or fell back.
#
# Usage: rs/parity.sh [case-name-regex]     REBUILD=1 rebuilds the sandbox template.
set -uo pipefail
source "$(dirname "$0")/sandbox.sh"
FILTER=${1:-.}
OUT=$ROOT/out
W=$HOME/Pictures/Wallpapers
DATA=$(cd "$RS_DIR/../src/caelestia/data" && pwd)
VIDEO=$W/Live/singularity-cosmic-vortex-moewalls-com.mp4

[[ -x $RS_BIN ]] || { echo "build first: cargo build --release" >&2; exit 1; }
ensure_template

# --- setup helpers (run against the live sandbox before each side) ---

state=$SB/state/caelestia
use_scheme() { # name flavour mode variant
    python3 -I - "$@" "$DATA" "$state/scheme.json" << 'EOF'
import json, sys
n, f, m, v, data, out = sys.argv[1:]
c = {}
for line in open(f"{data}/schemes/{n}/{f}/{m}.txt").read().splitlines():
    if line:
        k, val = line.split(" ")
        c[k.strip()] = val.strip().removeprefix("#")
json.dump({"name": n, "flavour": f, "mode": m, "variant": v, "colours": c}, open(out, "w"))
EOF
}
scheme_field() { sed -i -E "s/\"$1\": \"[^\"]*\"/\"$1\": \"$2\"/" "$state/scheme.json"; }
cfg_all() {
    cat > "$SB/config/caelestia/cli.json" << EOF
{"theme": {"enableChromium": true, "iconTheme": "Fancy-Icons",
  "postHook": "env | grep ^SCHEME_ | sort > $SB/theme-hook.txt; echo theme-hook-out"},
 "wallpaper": {"postHook": "env | grep -E '^(SCHEME_|WALLPAPER_|THUMBNAIL_)' | sort > $SB/wall-hook.txt; echo wall-hook-out"}}
EOF
}
cfg_bad() { echo '{"theme": ' > "$SB/config/caelestia/cli.json"; }
cfg_none() { rm "$SB/config/caelestia/cli.json"; }
no_hypr() { RUN_ENV=(HYPRLAND_INSTANCE_SIGNATURE=); }
hold_lock() { flock "$state/theme.lock" sleep 2 & sleep 0.1; }

# --- cases: name | expected Rust path (N native, F fallback, ? random) | setup | args ---
# "rand" in the name: random pick, so only exit code and stdout are compared.
cases=(
    "get|N||scheme get"
    "get-n|N||scheme get -n"
    "get-f|N||scheme get -f"
    "get-m|N||scheme get --mode"
    "get-v|N||scheme get -v"
    "get-all-flags|N||scheme get -nfmv"
    "get-no-scheme-json|N|rm $state/scheme.json|scheme get"
    "get-bad-scheme-json|N|echo '{bad' > $state/scheme.json|scheme get -n"
    "list|N||scheme list"
    "list-n|N||scheme list -n"
    "list-f|N||scheme list -f"
    "list-m|N||scheme list -m"
    "list-v|N||scheme list --variants"
    "list-multi|N||scheme list -n -v -m"
    "list-static-current|N|use_scheme caelestia default dark tonalspot|scheme list"
    # Dynamic colours under a static flavour ("medium") aren't cached; Python generates them
    "list-static-flavour-miss|F|use_scheme gruvbox medium light tonalspot|scheme list"
    "list-dynamic-no-thumb|N|rm $state/wallpaper/thumbnail.jpg|scheme list"
    "list-dynamic-cache-miss|F|scheme_field variant expressive|scheme list"
    "set-no-args|N||scheme set"
    "set-notify-no-args|N||scheme set --notify"
    "set-latte|N||scheme set -n catppuccin -f latte"
    "set-gruvbox-hard-light|N||scheme set -n gruvbox -f hard -m light"
    "set-rosepine|N||scheme set -n rosepine"
    "set-combined-forms|N||scheme set -nnord --mode=dark -v=vibrant"
    "set-from-static|N|use_scheme nord medium dark tonalspot|scheme set -n everforest -f soft"
    "set-shell-mode-hit|N|scheme_field variant vibrant|scheme set --notify -m light"
    "set-shell-mode-miss|F||scheme set --notify -m light"
    "set-same-mode|N||scheme set --notify -m dark"
    "set-variant-hit|N||scheme set -v vibrant"
    "set-variant-miss|F||scheme set -v expressive"
    "set-flavour-hard|N||scheme set -f hard"
    "set-flavour-mode-miss|F||scheme set -f hard -m light"
    "set-notify-bad-name|N||scheme set --notify -n notascheme"
    "set-notify-bad-flavour|N||scheme set --notify -f bogus"
    "set-notify-bad-mode|N|use_scheme nord medium dark tonalspot|scheme set --notify -m light"
    "set-notify-dynamic-no-wall|N|use_scheme catppuccin mocha dark tonalspot; rm $state/wallpaper/thumbnail.jpg|scheme set --notify -n dynamic"
    "set-dynamic-from-static|N|use_scheme catppuccin mocha dark tonalspot|scheme set -n dynamic"
    "set-partial-then-error|N||scheme set -n catppuccin -f bogus"
    "set-config-all-hooks|N|cfg_all|scheme set -n gruvbox"
    "set-config-bad-json|N|cfg_bad|scheme set -n gruvbox"
    "set-config-none|N|cfg_none|scheme set -n nord"
    "set-no-hyprland|N|no_hypr|scheme set -n nord"
    "set-lock-busy|N|hold_lock|scheme set -n nord"
    "set-rand|?||scheme set -r"
    "set-abbrev|F||scheme set --nam nord"
    "set-help|F||scheme set -h"
    "version|F||-v"
    "wall-print-current|N||wallpaper"
    "wall-no-current|N|rm $state/wallpaper/path.txt|wallpaper"
    "wall-p-current|N||wallpaper -p"
    "wall-p-jpg|N||wallpaper -p $W/b-139.jpg"
    "wall-p-png|N||wallpaper -p $W/tokyo_skylines.png"
    "wall-p-video|N||wallpaper -p $VIDEO"
    "wall-p-gif|N||wallpaper -p $SB/media/anim.gif"
    "wall-p-no-smart|N||wallpaper -p $W/b-139.jpg -N"
    "wall-p-uncached|F||wallpaper -p $SB/media/fresh.png"
    "wall-p-missing|F||wallpaper -p $SB/media/nope.jpg"
    "wall-f-jpg|N||wallpaper -f $W/b-139.jpg"
    "wall-f-same|N||wallpaper -f $W/wallhaven-rq6qp7.jpg"
    "wall-f-png-light|N||wallpaper -f $W/pink_explosion_light_4k.png"
    "wall-f-video|N||wallpaper -f $VIDEO"
    "wall-f-gif|N||wallpaper -f $SB/media/anim.gif"
    "wall-f-symlink|N||wallpaper -f $SB/media/link.jpg"
    "wall-f-no-smart|N||wallpaper -N -f $W/b-139.jpg"
    "wall-f-static-scheme|N|use_scheme gruvbox medium dark tonalspot|wallpaper -f $W/b-139.jpg"
    "wall-f-uncached|F||wallpaper -f $SB/media/fresh.png"
    "wall-f-not-image|N||wallpaper -f $SB/media/notes.txt"
    "wall-f-missing|N||wallpaper -f $SB/media/nope.jpg"
    "wall-f-config-all-hooks|N|cfg_all|wallpaper -f $W/b-139.jpg"
    "wall-f-config-bad-json|N|cfg_bad|wallpaper -f $W/b-139.jpg"
    "wall-r-no-filter|N|set_current_wall $SB/walls/pair/current.jpg|wallpaper -r $SB/walls/pair -n"
    "wall-r-filter-default-dir|N|set_current_wall $SB/pictures/Wallpapers/current.jpg|wallpaper -r"
    "wall-r-filter-video|N|set_current_wall $SB/walls/video/current.jpg|wallpaper -r $SB/walls/video"
    "wall-r-threshold|F|set_current_wall $SB/walls/pair/current.jpg|wallpaper -r $SB/walls/pair -t 0.5"
    "wall-r-threshold-no-filter|N|set_current_wall $SB/walls/pair/current.jpg|wallpaper -r $SB/walls/pair -n -t 0.5"
    "wall-r-empty|N||wallpaper -r $SB/walls/empty"
    "wall-r-only-current|N|set_current_wall $SB/walls/only/current.jpg|wallpaper -r $SB/walls/only -n"
    "wall-r-no-hyprland|F|no_hypr|wallpaper -r $SB/walls/pair"
    "wall-r-rand|?||wallpaper -r -n -N"
)

snapshot() { # dir -> listing of every entry, with sha256 for files
    (
        cd "$1" || exit
        find . -mindepth 1 -printf '%p %y %m %l\n' | sort
        find . -type f -print0 | sort -z | xargs -0 -r sha256sum
    )
}

run_side() { # side bin setup args
    local side=$1 bin=$2 setup=$3 args=$4 d=$OUT/$name
    reset_sandbox
    RUN_ENV=()
    eval "$setup"
    eval "set -- $args"
    sandbox_run "$bin" "$@" > "$d/$side.stdout" 2> "$d/$side.stderr"
    echo $? > "$d/$side.exit"
    sleep 0.25 # detached papirus-folders call
    sed -E 's/tmp[a-z0-9_]{8}w/tmpXXXXXXXXw/g' "$LOG" | sort > "$d/$side.log"
    snapshot "$SB" > "$d/$side.tree"
    [[ -s $FALLBACK_MARK ]] && echo F > "$d/$side.path" || echo N > "$d/$side.path"
}

rm -rf "$OUT"
mkdir -p "$OUT"
pass=0 fail=0 native=0 fellback=0
failed=()
for c in "${cases[@]}"; do
    IFS='|' read -r name expect setup args <<< "$c"
    [[ $name =~ $FILTER ]] || continue
    d=$OUT/$name
    mkdir -p "$d"
    run_side py "$PY_BIN" "$setup" "$args"
    rm -rf "$ROOT/sb.py"
    mv "$SB" "$ROOT/sb.py"
    run_side rs "$RS_BIN" "$setup" "$args"
    path=$(cat "$d/rs.path")
    if [[ $path == N ]]; then native=$((native + 1)); else fellback=$((fellback + 1)); fi

    problems=()
    cmp -s "$d/py.exit" "$d/rs.exit" || problems+=("exit $(cat "$d/py.exit") vs $(cat "$d/rs.exit")")
    cmp -s "$d/py.stdout" "$d/rs.stdout" || problems+=(stdout)
    if [[ $name != *rand* ]]; then
        cmp -s "$d/py.log" "$d/rs.log" || problems+=(commands)
        if ! cmp -s "$d/py.tree" "$d/rs.tree"; then
            problems+=(files)
            diff "$d/py.tree" "$d/rs.tree" > "$d/tree.diff"
            # content diffs for files present in both
            awk '$2 ~ /^[0-9a-f]{64}$/ {print $3}' "$d/tree.diff" | sort -u | while read -r f; do
                [[ -f $ROOT/sb.py/$f && -f $SB/$f ]] && diff -u "$ROOT/sb.py/$f" "$SB/$f" | head -20
            done > "$d/content.diff"
        fi
    fi
    [[ $expect != "?" && $expect != "$path" ]] && problems+=("expected $([[ $expect == N ]] && echo native || echo fallback)")
    if ((${#problems[@]})); then
        fail=$((fail + 1))
        failed+=("$name")
        printf 'FAIL %-30s %s  (%s)\n' "$name" "$path" "${problems[*]}"
    else
        pass=$((pass + 1))
        printf 'ok   %-30s %s\n' "$name" "$path"
    fi
done
rm -rf "$ROOT/sb.py"
echo
echo "cases: $((pass + fail))  passed: $pass  failed: $fail  rust native: $native  rust fallback: $fellback"
((fail)) && echo "failed: ${failed[*]}  (details in $OUT/<case>/)"
exit $((fail > 0))
