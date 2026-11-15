# Shared sandbox for parity.sh and bench.sh. Source it; don't run it.
#
# Every run happens inside `unshare -rm` with:
#   - an empty fake /dev/pts (inside the sandbox), so terminal colour sequences never reach real terminals
#   - $HOME bind-mounted read-only, so any stray write to the real home fails loudly
#   - XDG_* dirs, HOME, TMPDIR pointing into the sandbox
#   - logging shims first on PATH for every external command the theme code runs
# Hyprland stays reachable for the read-only `status` and `monitors` requests; nothing ported writes to it.

ROOT=${CAELESTIA_SANDBOX_ROOT:-/tmp/caelestia-cli-sandbox}
SB=$ROOT/sb # live sandbox; always the same path so symlink targets and file contents match between runs
TPL=$ROOT/tpl
BIN=$ROOT/bin
LOG=$ROOT/cmds.log
FALLBACK_MARK=$ROOT/fallback.mark
RS_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
RS_BIN=${RS_BIN:-$RS_DIR/target/release/caelestia}
PY_BIN=${PY_BIN:-$HOME/.local/bin/caelestia-py}
REAL_HOME=$HOME
WALLS=$HOME/Pictures/Wallpapers

write_shims() {
    rm -rf "$BIN"
    mkdir -p "$BIN"
    # One line per call: name, quoted args, and quoted stdin for the commands that read it
    cat > "$BIN/_shim" << 'EOF'
#!/bin/bash
n=${0##*/}
line="$n$(printf ' %q' "$@")"
if [[ $n == dconf || ($n == sudo && $2 == tee) ]]; then
    in=$(cat; echo .)
    line+=" <<< $(printf "%q" "${in%.}")"
fi
echo "$line" >> "$SHIM_LOG"
case $n in
    sass | ffmpeg | ffprobe) exec "/usr/bin/$n" "$@" ;;
esac
exit 0
EOF
    chmod +x "$BIN/_shim"
    for c in dconf killall sass notify-send sudo papirus-folders ffmpeg ffprobe chromium; do
        ln -s _shim "$BIN/$c"
    done
    # Python CLI wrapper: records that a fallback happened, then runs the real CLI
    cat > "$BIN/caelestia-py" << EOF
#!/bin/bash
echo fallback >> "$FALLBACK_MARK"
exec "$PY_BIN" "\$@"
EOF
    chmod +x "$BIN/caelestia-py"
}

# Run "$@" in the namespace with the sandbox environment. Extra env vars go in $RUN_ENV (array).
sandbox_run() {
    unshare -rm sh -c '
        mount --bind "$1/pts" /dev/pts && mount --bind "$2" "$2" && mount -o remount,bind,ro "$2" || exit 99
        shift 2
        exec "$@"' sh "$SB" "$REAL_HOME" \
        env -i \
        HOME="$SB/home" PATH="$BIN:/usr/bin" LANG=C.UTF-8 TMPDIR="$SB/tmp" \
        XDG_CONFIG_HOME="$SB/config" XDG_DATA_HOME="$SB/data" XDG_STATE_HOME="$SB/state" \
        XDG_CACHE_HOME="$SB/cache" XDG_PICTURES_DIR="$SB/pictures" XDG_VIDEOS_DIR="$SB/videos" \
        XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" HYPRLAND_INSTANCE_SIGNATURE="$HYPRLAND_INSTANCE_SIGNATURE" \
        SHIM_LOG="$LOG" CAELESTIA_PY="$BIN/caelestia-py" \
        "${RUN_ENV[@]}" "$@"
}

reset_sandbox() {
    rm -rf "$SB"
    cp -a "$TPL" "$SB"
    : > "$LOG"
    rm -f "$FALLBACK_MARK"
}

# Point the current wallpaper at $1 (path.txt, current link, thumbnail link), the way set_wallpaper leaves them.
# $2 is the tree to write into (default: the live sandbox); link targets always use the live sandbox path.
set_current_wall() {
    local wall=$1 dir=${2:-$SB}/state/caelestia/wallpaper hash
    hash=$(sha256sum < "$wall" | cut -d' ' -f1)
    mkdir -p "$dir"
    printf '%s' "$wall" > "$dir/path.txt"
    ln -sfn "$wall" "$dir/current"
    ln -sfn "$ROOT/sb/cache/caelestia/wallpapers/$hash/thumbnail.jpg" "$dir/thumbnail.jpg"
}

build_template() {
    echo ":: building sandbox template in $TPL" >&2
    rm -rf "$ROOT"
    mkdir -p "$ROOT"
    write_shims
    SB=$TPL # build in place
    mkdir -p "$SB"/{config/caelestia/templates,data,state/caelestia,cache/caelestia,pictures/Wallpapers,videos,tmp,pts,media}
    mkdir -p "$SB/home/.icons/Papirus" # makes papirus colour sync run, through the sudo shim
    touch "$SB/pts/0" "$SB/pts/1" "$SB/pts/ptmx"

    cp "$REAL_HOME/.config/caelestia/cli.json" "$SB/config/caelestia/cli.json"
    cp -a "$REAL_HOME/.cache/caelestia/wallpapers" "$REAL_HOME/.cache/caelestia/schemes" "$SB/cache/caelestia/"
    cp "$REAL_HOME/.local/state/caelestia/scheme.json" "$SB/state/caelestia/scheme.json"

    # User template exercising the dynamic {{ colour.format }} and {{ mode }} syntax, plus edge cases
    cat > "$SB/config/caelestia/templates/test.conf" << 'EOF'
hex={{ primary.hex }} alpha={{primary.hexalpha}} rgb={{ secondary.rgb }} rgba={{ secondary.rgbalpha }}
hsl={{ surface.hsl }} h={{ tertiary.hue }} s={{ tertiary.saturation }} l={{ tertiary.lightness }}
r={{ error.red }} g={{ error.green }} b={{ error.blue }} mode={{ mode }} m2={{	mode  }}
unknown={{ nope.hex }} badform={{ primary.nope }} three={{ a.b.c }} plain={{ x }} nested={{ {{ primary.hex }} }}
open={{ primary.hex } multi={{
mode
}} brace={{{ primary.hex }}}
EOF
    # Zed's theme as a symlink: apply_zed must replace it with a regular file
    mkdir -p "$SB/config/zed/themes"
    echo old > "$SB/config/zed/old.json"
    ln -s ../old.json "$SB/config/zed/themes/caelestia.json"

    # Media made from real wallpapers: an animated GIF, an uncached PNG, and tiny files the size filter drops
    local src=$WALLS/b-139.jpg
    ffmpeg -v error -loop 1 -i "$src" -vf "scale=480:-1" -frames:v 3 -r 3 "$SB/media/anim.gif"
    ffmpeg -v error -i "$src" -vf "scale=1800:-1,hflip" "$SB/media/fresh.png"
    ffmpeg -v error -i "$src" -vf "scale=100:100" "$SB/media/tiny.png"
    ffmpeg -v error -f lavfi -i "color=c=red:s=160x90:d=1" -pix_fmt yuv420p "$SB/media/tiny.mp4"
    ln -s "$WALLS/b-097.jpg" "$SB/media/link.jpg"
    echo "not an image" > "$SB/media/notes.txt"

    # Dirs for `wallpaper -r`. Copies, not symlinks, so the "skip the current wallpaper" check can match.
    local w=$SB/pictures/Wallpapers
    cp "$WALLS/wallhaven-rq6qp7.jpg" "$w/current.jpg"
    cp "$WALLS/b-139.jpg" "$w/b-139.jpg"
    cp "$SB/media/tiny.png" "$w/tiny.png"
    cp "$SB/media/tiny.mp4" "$w/tiny.mp4"
    cp "$SB/media/notes.txt" "$w/notes.txt"
    mkdir -p "$w/sub"
    cp "$SB/media/tiny.png" "$w/sub/tiny2.png"
    mkdir -p "$SB/walls/pair" "$SB/walls/video" "$SB/walls/only" "$SB/walls/empty"
    cp "$w/current.jpg" "$SB/walls/pair/current.jpg"
    cp "$WALLS/b-097.jpg" "$SB/walls/pair/b-097.jpg"
    cp "$w/notes.txt" "$SB/walls/pair/notes.txt"
    cp "$w/current.jpg" "$SB/walls/video/current.jpg"
    cp "$w/tiny.png" "$SB/walls/video/tiny.png"
    ln -s "$WALLS/Live/singularity-cosmic-vortex-moewalls-com.mp4" "$SB/walls/video/vortex.mp4"
    cp "$w/current.jpg" "$SB/walls/only/current.jpg"

    # Warm caches the way the Python CLI would, so the matching cases can run natively.
    # Only cache/ is kept; state and generated theme files are rebuilt below.
    local keep_cfg=$SB/config/caelestia/cli.json
    echo '{"theme": {"enableTerm": false, "enableHypr": false, "enableDiscord": false, "enableGtk": false, "enableQt": false, "enableChromium": false}}' > "$keep_cfg.warm"
    mv "$keep_cfg" "$keep_cfg.real"
    mv "$keep_cfg.warm" "$keep_cfg"
    for args in \
        "wallpaper -f $SB/media/anim.gif" \
        "wallpaper -p $SB/media/anim.gif" \
        "wallpaper -f $WALLS/Live/singularity-cosmic-vortex-moewalls-com.mp4" \
        "wallpaper -p $WALLS/Live/singularity-cosmic-vortex-moewalls-com.mp4" \
        "wallpaper -f $WALLS/pink_explosion_light_4k.png" \
        "wallpaper -f $WALLS/b-139.jpg" \
        "wallpaper -f $WALLS/wallhaven-rq6qp7.jpg" \
        "scheme set -n dynamic -v vibrant" \
        "scheme set -m light" \
        "scheme set -v tonalspot -m dark" \
        "scheme set -f hard"; do
        # shellcheck disable=SC2086
        sandbox_run "$PY_BIN" $args > /dev/null || echo ":: warm-up failed: $args" >&2
    done
    mv "$keep_cfg.real" "$keep_cfg"

    # Back to a clean starting state: real scheme, current wallpaper, no generated theme files
    cp "$REAL_HOME/.local/state/caelestia/scheme.json" "$SB/state/caelestia/scheme.json"
    rm -rf "$SB/state/caelestia/sequences.txt" "$SB/config/hypr" "$SB/config/spicetify" "$SB/config/btop" \
        "$SB/config/cava" "$SB/data/PandoraLauncher" "$SB/data/warp-terminal" "$SB/config/nvtop" \
        "$SB/config/htop" "$SB/config/fuzzel" "$SB/state/caelestia/theme"
    rm -f "$SB/config/zed/themes/caelestia.json"
    ln -s ../old.json "$SB/config/zed/themes/caelestia.json"
    set_current_wall "$WALLS/wallhaven-rq6qp7.jpg" "$TPL"
    : > "$SB/pts/0"
    : > "$SB/pts/1"
    SB=$ROOT/sb
    : > "$LOG"
}

ensure_template() {
    [[ -d $TPL && -z ${REBUILD:-} ]] || build_template
    write_shims # every run, so a changed $PY_BIN takes effect without a rebuild
}
