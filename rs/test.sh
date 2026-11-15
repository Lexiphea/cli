#!/bin/bash
# End-to-end checks of the caelestia binary in the sandbox (see sandbox.sh): cold and warm caches,
# concurrent changes and command-line errors.
#
# Usage: rs/test.sh     REBUILD=1 rebuilds the sandbox template.
set -uo pipefail
source "$(dirname "$0")/sandbox.sh"
W=$HOME/Pictures/Wallpapers
state=$SB/state/caelestia

[[ -x $RS_BIN ]] || { echo "build first: cargo build --release" >&2; exit 1; }
ensure_template

pass=0 fail=0
check() { # name, then a command that must succeed
    local name=$1
    shift
    if "$@"; then
        pass=$((pass + 1))
        printf 'ok   %s\n' "$name"
    else
        fail=$((fail + 1))
        printf 'FAIL %s\n' "$name"
    fi
}
run() { sandbox_run "$RS_BIN" "$@" > "$ROOT/stdout" 2> "$ROOT/stderr"; echo $? > "$ROOT/exit"; }
exit_is() { [[ $(cat "$ROOT/exit") == "$1" ]]; }
stdout_has() { grep -q -- "$1" "$ROOT/stdout"; }
# Cache dir of a still image
wall_cache() { echo "$SB/cache/caelestia/wallpapers/$(sha256sum < "$1" | cut -d' ' -f1)"; }
# scheme.json and the Hyprland theme carry the same colours
consistent() {
    python3 -I - "$state/scheme.json" "$SB/config/hypr/scheme/current.lua" << 'PY'
import json, re, sys
bg = json.load(open(sys.argv[1]))["colours"]["background"]
sys.exit(re.search(r'background = "(\w+)"', open(sys.argv[2]).read()).group(1) != bg)
PY
}
scheme_is() { [[ $(python3 -I -c 'import json, sys; s = json.load(open(sys.argv[1])); print(s["name"], s["variant"])' "$state/scheme.json") == "$1" ]]; }

reset_sandbox
run wallpaper -f "$SB/media/fresh.png"
c=$(wall_cache "$SB/media/fresh.png")
check "cold wallpaper -f: caches filled (thumbnail, smart options, colours), then applied" \
    eval 'exit_is 0 && [[ -s $c/thumbnail.jpg && -s $c/smart.json ]] && [[ $(cat "$state/wallpaper/path.txt") == "$SB/media/fresh.png" ]] && consistent'
run wallpaper -f "$SB/media/fresh.png"
check "warm wallpaper -f" eval 'exit_is 0 && consistent'

reset_sandbox
run scheme set -v expressive
check "uncached variant: colours generated, then applied" eval 'exit_is 0 && scheme_is "dynamic expressive" && consistent'

reset_sandbox
run wallpaper -p "$SB/media/anim.gif"
check "wallpaper -p on a GIF prints a scheme" eval 'exit_is 0 && stdout_has "\"colours\""'

reset_sandbox
for _ in 1 2 3; do
    sandbox_run "$RS_BIN" scheme set -n catppuccin -f mocha > /dev/null &
    sandbox_run "$RS_BIN" scheme set -n gruvbox -f hard > /dev/null &
    wait
    consistent || break
done
check "concurrent scheme set: state and themes match" consistent

reset_sandbox
run wallpaper -r "$SB/walls/pair" -t 0.5
check "wallpaper -r -t works" eval 'exit_is 0 && consistent'
RUN_ENV=(HYPRLAND_INSTANCE_SIGNATURE=)
run wallpaper -r "$SB/walls/pair"
RUN_ENV=()
check "wallpaper -r without Hyprland: clear error" eval 'exit_is 1 && grep -q "use -n" "$ROOT/stderr"'

reset_sandbox
run scheme set --nam nord
check "abbreviated option: usage error" eval 'exit_is 2 && grep -q "unrecognized arguments: --nam" "$ROOT/stderr"'
run wallpaper -f
check "missing value: usage error" eval 'exit_is 2 && grep -q "expected one argument" "$ROOT/stderr"'
run wallpaper -h
check "wallpaper -h prints help" eval 'exit_is 0 && stdout_has "usage: caelestia wallpaper"'
run --version
check "--version" eval 'exit_is 0 && stdout_has "^caelestia-cli 1"'

echo
echo "passed: $pass  failed: $fail"
exit $((fail > 0))
