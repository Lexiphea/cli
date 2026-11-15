//! Wallpapers, ported from the Python CLI's `utils/wallpaper.py` and `subcommands/wallpaper.py`.
//! Thumbnails, first frames and smart options need Pillow, so Python generates them on a cache miss.

use crate::scheme::{Ctx, Effect, Stop, fail, generate};
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

fn not_valid(wall: &Path) -> Stop {
    fail(format!("\"{}\" is not a valid image or video", wall.display()))
}

/// The wallpaper's cache dir, holding its thumbnail and smart.json; under the first frame's hash
/// for animated ones. Python fills in whatever is missing.
fn cache_dir(ctx: &Ctx, wall: &Path) -> Result<PathBuf, Stop> {
    let dir = |hash: String| ctx.p.c_cache.join("wallpapers").join(hash);
    let find = || -> Option<PathBuf> {
        let mut hash = hash_file(wall).ok()?;
        if is_animated(wall) {
            hash = hash_file(&dir(hash).join("first_frame.png")).ok()?;
        }
        let d = dir(hash);
        (d.join("thumbnail.jpg").exists() && d.join("smart.json").exists()).then_some(d)
    };
    if let Some(d) = find() {
        return Ok(d);
    }
    generate(&["wallpaper".as_ref(), wall.as_os_str()])?;
    find().ok_or_else(|| fail(format!("no thumbnail was generated for {}", wall.display())))
}

/// `get_smart_opts` from smart.json: (mode, variant).
fn smart_opts(cache: &Path) -> Result<(String, String), Stop> {
    let path = cache.join("smart.json");
    let v: Option<Value> = fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok());
    let field = |k| v.as_ref()?.get(k)?.as_str().map(str::to_owned);
    field("mode").zip(field("variant")).ok_or_else(|| fail(format!("invalid {}", path.display())))
}

/// `wallpaper -p`: print the scheme a wallpaper would give.
pub fn print_colours(ctx: &mut Ctx, wall: &str, no_smart: bool) -> Result<(), Stop> {
    ctx.load_scheme()?;
    let wall = Path::new(wall);
    if !is_valid(wall) {
        return Err(not_valid(wall));
    }
    let cache = cache_dir(ctx, wall)?;
    let cur = ctx.cur();
    let (mode, variant) = if no_smart { (cur.mode.clone(), cur.variant.clone()) } else { smart_opts(&cache)? };
    let flavour = cur.flavour.clone();
    let thumb = cache.join("thumbnail.jpg");
    let colours = ctx.image_colours(&thumb, &variant, &flavour, &mode)?;
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
    let wall = fs::canonicalize(wall).map_err(|_| not_valid(wall))?;
    if !is_valid(&wall) {
        return Err(not_valid(&wall));
    }
    let wall_str = wall.to_str().ok_or_else(|| fail(format!("{} is not UTF-8", wall.display())))?.to_owned();
    let cache = cache_dir(ctx, &wall)?;
    let dir = ctx.p.wall_dir();
    ctx.effects.push(Effect::Text(dir.join("path.txt"), wall_str.clone()));
    ctx.effects.push(Effect::Link(dir.join("current"), wall.clone()));
    let thumb = cache.join("thumbnail.jpg");
    ctx.effects.push(Effect::Link(dir.join("thumbnail.jpg"), thumb.clone()));
    ctx.thumb = thumb.clone();

    ctx.load_scheme()?;
    if ctx.cur().name == "dynamic" && !no_smart {
        let (mode, variant) = smart_opts(&cache)?;
        ctx.set_mode(&mode)?;
        ctx.set_variant(&variant)?;
    }
    ctx.update()?;
    Ok(WallSet { wall: wall_str, thumb: thumb.to_string_lossy().into_owned() })
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
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
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
    /// Minimum share of the smallest monitor's size
    pub threshold: f64,
    pub no_smart: bool,
}

/// `set_random`, up to `apply_colours`.
pub fn random(ctx: &mut Ctx, a: &RandomArgs) -> Result<WallSet, Stop> {
    let dir = Path::new(a.dir);
    let mut walls = Vec::new();
    if dir.is_dir() {
        find_walls(dir, &mut walls);
        if !a.no_filter {
            let no_size = |e: String| {
                fail(format!("cannot get the monitor size from Hyprland ({e}); use -n to skip the size filter"))
            };
            let monitors = hypr_json("monitors").map_err(|e| no_size(e.to_string()))?;
            let dim = |k: &str| -> Result<i64, Stop> {
                let ms = monitors.as_array().ok_or_else(|| no_size("not a list".into()))?;
                let vals: Option<Vec<i64>> = ms.iter().map(|m| m.get(k).and_then(Value::as_i64)).collect();
                vals.and_then(|v| v.into_iter().min()).ok_or_else(|| no_size(format!("no {k}")))
            };
            let (min_w, min_h) = (dim("width")? as f64 * a.threshold, dim("height")? as f64 * a.threshold);
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
        Err(e) => return Err(fail(format!("cannot read the current wallpaper: {e}"))),
    }
    let choice = walls.swap_remove(rand_below(walls.len()));
    set(ctx, &choice, a.no_smart)
}

/// `get_wallpaper()`
pub fn current(ctx: &Ctx) -> Result<Option<String>, Stop> {
    match fs::read(ctx.p.wall_dir().join("path.txt")) {
        Ok(b) => String::from_utf8(b).map(Some).map_err(|_| fail("the current wallpaper's path is not UTF-8")),
        Err(_) => Ok(None),
    }
}
