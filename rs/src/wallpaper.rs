//! Wallpapers: `utils/wallpaper.py` and `subcommands/wallpaper.py`, for cached wallpapers.
//! Thumbnails, first frames and smart options that aren't cached yet need Pillow, so Python.

use crate::scheme::{Ctx, Effect, Stop};
use crate::util::{dumps, hash_file, hypr_json, rand_below};
use serde_json::{Map, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const IMAGE_EXT: [&str; 7] = [".jpg", ".jpeg", ".png", ".webp", ".tif", ".tiff", ".gif"];
const VIDEO_EXT: [&str; 5] = [".mp4", ".mkv", ".webm", ".mov", ".avi"];

/// `Path.suffix.lower()`
fn suffix(p: &Path) -> String {
    p.extension().map_or_else(String::new, |e| format!(".{}", e.to_string_lossy().to_lowercase()))
}

fn is_video(p: &Path) -> bool {
    VIDEO_EXT.contains(&suffix(p).as_str())
}

fn is_animated(p: &Path) -> bool {
    suffix(p) == ".gif" || is_video(p)
}

fn is_valid(p: &Path) -> bool {
    let s = suffix(p);
    p.is_file() && (IMAGE_EXT.contains(&s.as_str()) || VIDEO_EXT.contains(&s.as_str()))
}

fn wall_cache(ctx: &Ctx, hash: &str) -> PathBuf {
    ctx.p.c_cache.join("wallpapers").join(hash)
}

fn hash(p: &Path) -> Result<String, Stop> {
    hash_file(p).map_err(|_| Stop::Fallback)
}

/// A cached file Python would otherwise generate.
fn cached(p: PathBuf) -> Result<PathBuf, Stop> {
    if p.exists() { Ok(p) } else { Err(Stop::Fallback) }
}

/// `get_smart_opts` from smart.json: (mode, variant).
fn smart_opts(cache: &Path) -> Result<(String, String), Stop> {
    let text = fs::read_to_string(cache.join("smart.json")).map_err(|_| Stop::Fallback)?;
    let v: Value = serde_json::from_str(&text).map_err(|_| Stop::Fallback)?;
    let field = |k| v.get(k).and_then(Value::as_str).map(str::to_owned).ok_or(Stop::Fallback);
    Ok((field("mode")?, field("variant")?))
}

/// `wallpaper -p`: print the scheme a wallpaper would give.
pub fn print_colours(ctx: &mut Ctx, wall: &str, no_smart: bool) -> Result<(), Stop> {
    ctx.load_scheme()?;
    let wall = Path::new(wall);
    let cache = wall_cache(ctx, &hash(wall)?);
    // The first frame only matters if smart.json or the thumbnail need generating, but Python makes it first
    if is_animated(wall) {
        cached(cache.join("first_frame.png"))?;
    }
    let cur = ctx.cur();
    let (mode, variant) = if no_smart { (cur.mode.clone(), cur.variant.clone()) } else { smart_opts(&cache)? };
    let flavour = cur.flavour.clone();
    let thumb = cached(cache.join("thumbnail.jpg"))?;
    let colours = ctx.image_colours(&thumb, &variant, &flavour, &mode).map_err(|_| Stop::Fallback)?;
    let mut m = Map::new();
    m.insert("name".into(), "dynamic".into());
    m.insert("flavour".into(), flavour.into());
    m.insert("mode".into(), mode.into());
    m.insert("variant".into(), variant.into());
    m.insert("colours".into(), Value::Object(colours));
    ctx.effects.push(Effect::Print(dumps(&Value::Object(m))));
    Ok(())
}

/// What `set_wallpaper` leaves for after the effects: the post hook needs these.
pub struct WallSet {
    pub wall: String,
    pub thumb: String,
}

/// `set_wallpaper`, up to (not including) `apply_colours`.
pub fn set(ctx: &mut Ctx, wall: &Path, no_smart: bool) -> Result<WallSet, Stop> {
    // Path.resolve(): a path that doesn't resolve isn't a valid wallpaper either
    let wall = match fs::canonicalize(wall) {
        Ok(w) => w,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(Stop::Fail(format!("\"{}\" is not a valid image or video", wall.display())));
        }
        Err(_) => return Err(Stop::Fallback),
    };
    if !is_valid(&wall) {
        return Err(Stop::Fail(format!("\"{}\" is not a valid image or video", wall.display())));
    }
    let wall_str = wall.to_str().ok_or(Stop::Fallback)?.to_owned();
    let mut src_hash = hash(&wall)?;
    if is_animated(&wall) {
        let frame = cached(wall_cache(ctx, &src_hash).join("first_frame.png"))?;
        src_hash = hash(&frame)?;
    }
    let dir = ctx.p.wall_dir();
    ctx.effects.push(Effect::Text(dir.join("path.txt"), wall_str.clone()));
    ctx.effects.push(Effect::Link(dir.join("current"), wall.clone()));
    let cache = wall_cache(ctx, &src_hash);
    let thumb = cached(cache.join("thumbnail.jpg"))?;
    ctx.effects.push(Effect::Link(dir.join("thumbnail.jpg"), thumb.clone()));
    ctx.thumb = thumb.clone();

    ctx.load_scheme()?;
    if ctx.cur().name == "dynamic" && !no_smart {
        let (mode, variant) = smart_opts(&cache)?;
        ctx.set_mode(&mode)?;
        ctx.set_variant(&variant)?;
    }
    ctx.update()?;
    Ok(WallSet { wall: wall_str, thumb: thumb.to_str().ok_or(Stop::Fallback)?.to_owned() })
}

/// `get_size` for the size filter. None means Python's check_wall would return False.
fn size(wall: &Path) -> Option<(i64, i64)> {
    if !is_video(wall) {
        return imagesize::size(wall).ok().map(|s| (s.width as i64, s.height as i64));
    }
    let o = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .args(["stream=width,height:stream_side_data=rotation", "-of", "default=nw=1:nk=1"])
        .arg(wall)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    let nums: Vec<i64> =
        String::from_utf8(o.stdout).ok()?.split_whitespace().map(|t| t.parse().ok()).collect::<Option<_>>()?;
    match nums[..] {
        [w, h, r, ..] if r % 180 != 0 => Some((h, w)),
        [w, h, ..] => Some((w, h)),
        _ => None,
    }
}

/// `directory.rglob("*")` filtered by `is_valid_wallpaper`: symlinked files count, symlinked dirs aren't entered.
fn find_walls(dir: &Path, walls: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            find_walls(&p, walls);
        } else if is_valid(&p) {
            walls.push(p);
        }
    }
}

pub struct RandomArgs<'a> {
    pub dir: &'a str,
    pub no_filter: bool,
    /// `-t` was given: Python then compares an int with a str and crashes, so leave that to Python
    pub threshold_given: bool,
    pub no_smart: bool,
}

/// `set_random`, up to `apply_colours`.
pub fn random(ctx: &mut Ctx, a: &RandomArgs) -> Result<WallSet, Stop> {
    let dir = Path::new(a.dir);
    let mut walls = Vec::new();
    if dir.is_dir() {
        find_walls(dir, &mut walls);
        if !a.no_filter {
            if a.threshold_given {
                return Err(Stop::Fallback);
            }
            let monitors = hypr_json("monitors").map_err(|_| Stop::Fallback)?;
            let dim = |k: &str| -> Result<i64, Stop> {
                let ms = monitors.as_array().ok_or(Stop::Fallback)?;
                let vals: Option<Vec<i64>> = ms.iter().map(|m| m.get(k).and_then(Value::as_i64)).collect();
                vals.and_then(|v| v.into_iter().min()).ok_or(Stop::Fallback)
            };
            let (min_w, min_h) = (dim("width")? as f64 * 0.8, dim("height")? as f64 * 0.8);
            walls.retain(|w| size(w).is_some_and(|(width, height)| width as f64 >= min_w && height as f64 >= min_h));
        }
    }
    if walls.is_empty() {
        return Err(Stop::Fail("No valid wallpapers found".into()));
    }
    match fs::read_to_string(ctx.p.wall_dir().join("path.txt")) {
        Ok(last) => {
            if let Some(i) = walls.iter().position(|w| w == Path::new(&last)) {
                walls.remove(i);
                if walls.is_empty() {
                    // Python's own `except ValueError` swallows "Only valid wallpaper is current",
                    // then random.choice([]) raises IndexError
                    return Err(Stop::Fail("Cannot choose from an empty sequence".into()));
                }
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(Stop::Fallback),
    }
    let choice = walls.swap_remove(rand_below(walls.len()));
    set(ctx, &choice, a.no_smart)
}

/// `get_wallpaper()`
pub fn current(ctx: &Ctx) -> Result<Option<String>, Stop> {
    match fs::read(ctx.p.wall_dir().join("path.txt")) {
        Ok(b) => String::from_utf8(b).map(Some).map_err(|_| Stop::Fallback),
        Err(_) => Ok(None),
    }
}
