//! Colour schemes: `utils/scheme.py`, the cache-reading half of `utils/material/__init__.py`,
//! and `subcommands/scheme.py`.
//!
//! Commands first run as a side-effect-free plan. Writes, symlinks, notifications and prints are
//! recorded as [`Effect`]s and only run once the plan finished without needing Python.

use crate::util::{
    Colours, Paths, char_slice, dumps, file_name, hash_file, py_int16, py_repr_list, rand_below, read_text,
};
use serde_json::{Map, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const VARIANTS: [&str; 9] =
    ["tonalspot", "vibrant", "expressive", "fidelity", "fruitsalad", "monochrome", "neutral", "rainbow", "content"];

const NO_WALL: &str =
    "No wallpaper set. Please set a wallpaper via `caelestia wallpaper` before setting a dynamic scheme.";

/// Why a plan stopped early.
#[derive(Debug)]
pub enum Stop {
    /// Needs work that isn't ported, or state the port doesn't model: exec the Python CLI.
    Fallback,
    /// Python raises here (exit 1). Effects recorded so far still run, as they would have in Python.
    Fail(String),
    /// A FileNotFoundError that some callers turn into a ValueError.
    Missing,
}

pub enum Effect {
    /// `atomic_write`
    Write(PathBuf, String),
    /// `Path.write_text`
    Text(PathBuf, String),
    /// unlink, then symlink `.0` -> `.1`
    Link(PathBuf, PathBuf),
    Notify(Vec<String>),
    Print(String),
}

#[derive(Clone)]
pub struct Scheme {
    pub name: String,
    pub flavour: String,
    pub mode: String,
    pub variant: String,
    pub colours: Colours,
    pub notify: bool,
}

impl Scheme {
    pub fn to_json(&self) -> String {
        let mut m = Map::new();
        m.insert("name".into(), self.name.clone().into());
        m.insert("flavour".into(), self.flavour.clone().into());
        m.insert("mode".into(), self.mode.clone().into());
        m.insert("variant".into(), self.variant.clone().into());
        m.insert("colours".into(), Value::Object(self.colours.clone()));
        dumps(&Value::Object(m))
    }
}

/// Plan state: Python's global `get_scheme()` object plus the recorded effects.
pub struct Ctx {
    pub p: Paths,
    scheme: Option<Scheme>,
    /// The thumbnail dynamic colours come from (`wallpaper/thumbnail.jpg`, or the one `wallpaper -f` is about to link)
    pub thumb: PathBuf,
    pub effects: Vec<Effect>,
}

/// A JSON object whose values are all strings, as colours must be.
pub fn as_colours(v: Value) -> Option<Colours> {
    match v {
        Value::Object(m) if m.values().all(Value::is_string) => Some(m),
        _ => None,
    }
}

fn str_field(m: &Map<String, Value>, k: &str) -> Result<String, Stop> {
    m.get(k).and_then(Value::as_str).map(str::to_owned).ok_or(Stop::Fallback)
}

/// `read_colours_from_file`. Malformed lines are a ValueError in Python.
fn read_colours(path: &Path) -> Result<Colours, Stop> {
    let text = read_text(path).map_err(|_| Stop::Fallback)?;
    let mut c = Colours::new();
    for line in splitlines(&text).into_iter().filter(|l| !l.is_empty()) {
        let parts: Vec<&str> = line.split(' ').collect();
        let [k, v] = parts[..] else {
            return Err(Stop::Fail(format!("bad colour line in {}: {line:?}", path.display())));
        };
        let v = v.trim();
        c.insert(k.trim().to_owned(), v.strip_prefix('#').unwrap_or(v).to_owned().into());
    }
    Ok(c)
}

/// `str.splitlines()`
fn splitlines(s: &str) -> Vec<&str> {
    let mut v = Vec::new();
    let mut start = 0;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}') {
            v.push(&s[start..i]);
            start = i + c.len_utf8();
            if c == '\r' && it.peek().is_some_and(|&(_, n)| n == '\n') {
                it.next();
                start += 1;
            }
        }
    }
    if start < s.len() {
        v.push(&s[start..]);
    }
    v
}

fn list_dir(dir: &Path, want_dir: bool) -> Result<Vec<String>, Stop> {
    let mut v = Vec::new();
    for e in fs::read_dir(dir).map_err(|_| Stop::Fallback)? {
        let p = e.map_err(|_| Stop::Fallback)?.path();
        if if want_dir { p.is_dir() } else { p.is_file() } {
            let name = if want_dir { file_name(&p) } else { p.file_stem().and_then(|s| s.to_str()) };
            v.push(name.ok_or(Stop::Fallback)?.to_owned());
        }
    }
    Ok(v)
}

fn pick(v: &[String]) -> Result<String, Stop> {
    if v.is_empty() { Err(Stop::Fallback) } else { Ok(v[rand_below(v.len())].clone()) }
}

impl Ctx {
    pub fn new(p: Paths) -> Self {
        let thumb = p.wall_dir().join("thumbnail.jpg");
        Ctx { p, scheme: None, thumb, effects: Vec::new() }
    }

    pub fn cur(&self) -> &Scheme {
        self.scheme.as_ref().expect("scheme loaded")
    }

    fn cur_mut(&mut self) -> &mut Scheme {
        self.scheme.as_mut().expect("scheme loaded")
    }

    /// `get_scheme()`: scheme.json, or the default scheme (saved) if it's missing or invalid JSON.
    pub fn load_scheme(&mut self) -> Result<(), Stop> {
        if self.scheme.is_some() {
            return Ok(());
        }
        let parsed = match fs::read(self.p.scheme_path()) {
            Ok(b) => {
                let s = String::from_utf8(b).map_err(|_| Stop::Fallback)?;
                serde_json::from_str::<Value>(&s).ok()
            }
            Err(_) => None,
        };
        let scheme = match parsed {
            Some(Value::Object(m)) => Scheme {
                name: str_field(&m, "name")?,
                flavour: str_field(&m, "flavour")?,
                mode: str_field(&m, "mode")?,
                variant: str_field(&m, "variant")?,
                colours: m.get("colours").cloned().and_then(as_colours).ok_or(Stop::Fallback)?,
                notify: false,
            },
            Some(_) => return Err(Stop::Fallback),
            None => {
                let s = Scheme {
                    colours: read_colours(&self.colours_path("catppuccin", "mocha", "dark"))?,
                    name: "catppuccin".into(),
                    flavour: "mocha".into(),
                    mode: "dark".into(),
                    variant: "tonalspot".into(),
                    notify: false,
                };
                self.effects.push(Effect::Write(self.p.scheme_path(), s.to_json()));
                s
            }
        };
        self.scheme = Some(scheme);
        Ok(())
    }

    fn schemes_dir(&self) -> PathBuf {
        self.p.cli_data.join("schemes")
    }

    pub fn names(&self) -> Result<Vec<String>, Stop> {
        let mut v = list_dir(&self.schemes_dir(), true)?;
        v.push("dynamic".into());
        Ok(v)
    }

    pub fn flavours(&self, name: &str) -> Result<Vec<String>, Stop> {
        if name == "dynamic" {
            return Ok(vec!["default".into(), "hard".into()]);
        }
        list_dir(&self.schemes_dir().join(name), true)
    }

    pub fn modes(&self, name: &str, flavour: &str) -> Result<Vec<String>, Stop> {
        if name == "dynamic" {
            return Ok(vec!["light".into(), "dark".into()]);
        }
        list_dir(&self.schemes_dir().join(name).join(flavour), false)
    }

    fn colours_path(&self, name: &str, flavour: &str, mode: &str) -> PathBuf {
        self.schemes_dir().join(name).join(flavour).join(mode).with_extension("txt")
    }

    /// `material.get_colours_for_image`, cache hits only: a miss needs colour generation, so Python.
    pub fn image_colours(&self, image: &Path, variant: &str, flavour: &str, mode: &str) -> Result<Colours, Stop> {
        let hash = hash_file(image)
            .map_err(|e| if e.kind() == io::ErrorKind::NotFound { Stop::Missing } else { Stop::Fallback })?;
        let cache =
            self.p.c_cache.join("schemes").join(hash).join(variant).join(flavour).join(mode).with_extension("json");
        let text = fs::read_to_string(cache).map_err(|_| Stop::Fallback)?;
        serde_json::from_str(&text).ok().and_then(as_colours).ok_or(Stop::Fallback)
    }

    fn notify(&mut self, title: &str, body: String) {
        if self.cur().notify {
            self.effects.push(Effect::Notify(vec!["-u".into(), "critical".into(), title.into(), body]));
        }
    }

    fn save(&mut self) {
        self.effects.push(Effect::Write(self.p.scheme_path(), self.cur().to_json()));
    }

    /// `Scheme._update_colours`
    fn refresh(&mut self) -> Result<(), Stop> {
        let s = self.cur();
        let colours = if s.name == "dynamic" {
            match self.image_colours(&self.thumb, &s.variant, &s.flavour, &s.mode) {
                Err(Stop::Missing) => {
                    self.notify("Unable to set dynamic scheme", NO_WALL.into());
                    return Err(Stop::Fail(NO_WALL.into()));
                }
                r => r?,
            }
        } else {
            read_colours(&self.colours_path(&s.name, &s.flavour, &s.mode))?
        };
        self.cur_mut().colours = colours;
        Ok(())
    }

    /// `Scheme.update_colours`
    pub fn update(&mut self) -> Result<(), Stop> {
        self.refresh()?;
        self.save();
        Ok(())
    }

    fn check_mode(&mut self) -> Result<(), Stop> {
        let s = self.cur();
        let modes = self.modes(&s.name, &s.flavour)?;
        if !modes.contains(&s.mode) {
            self.cur_mut().mode = modes.first().ok_or(Stop::Fallback)?.clone();
        }
        Ok(())
    }

    /// The `name` setter. argparse `choices` already rejected unknown names, so its notify branch can't run.
    fn set_name(&mut self, name: &str) -> Result<(), Stop> {
        if name == self.cur().name {
            return Ok(());
        }
        self.cur_mut().name = name.into();
        let flavours = self.flavours(name)?;
        if !flavours.contains(&self.cur().flavour) {
            self.cur_mut().flavour = flavours.first().ok_or(Stop::Fallback)?.clone();
        }
        self.check_mode()?;
        self.refresh()?;
        self.save();
        Ok(())
    }

    fn set_flavour(&mut self, flavour: &str) -> Result<(), Stop> {
        if flavour == self.cur().flavour {
            return Ok(());
        }
        let flavours = self.flavours(&self.cur().name)?;
        if !flavours.iter().any(|f| f == flavour) {
            let list = py_repr_list(&flavours);
            let body = format!(
                "\"{flavour}\" is not a valid flavour of scheme \"{}\".\nValid flavours are: {list}",
                self.cur().name
            );
            self.notify("Unable to set scheme flavour", body);
            return Err(Stop::Fail(format!("Invalid scheme flavour: \"{flavour}\". Valid flavours: {list}")));
        }
        self.cur_mut().flavour = flavour.into();
        self.check_mode()?;
        self.update()
    }

    pub fn set_mode(&mut self, mode: &str) -> Result<(), Stop> {
        if mode == self.cur().mode {
            return Ok(());
        }
        let s = self.cur();
        let modes = self.modes(&s.name, &s.flavour)?;
        if !modes.iter().any(|m| m == mode) {
            let body = format!("Scheme \"{} {}\" does not have a {mode} mode.", s.name, s.flavour);
            self.notify("Unable to set scheme mode", body);
            return Err(Stop::Fail(format!("Invalid scheme mode: \"{mode}\". Valid modes: {}", py_repr_list(&modes))));
        }
        self.cur_mut().mode = mode.into();
        self.update()
    }

    pub fn set_variant(&mut self, variant: &str) -> Result<(), Stop> {
        if variant == self.cur().variant {
            return Ok(());
        }
        self.cur_mut().variant = variant.into();
        self.update()
    }

    fn set_random(&mut self) -> Result<(), Stop> {
        let name = pick(&self.names()?)?;
        let flavour = pick(&self.flavours(&name)?)?;
        let mode = pick(&self.modes(&name, &flavour)?)?;
        let s = self.cur_mut();
        (s.name, s.flavour, s.mode) = (name, flavour, mode);
        self.update()
    }

    fn print(&mut self, s: String) {
        self.effects.push(Effect::Print(s));
    }
}

pub struct SetArgs {
    pub notify: bool,
    pub random: bool,
    pub name: Option<String>,
    pub flavour: Option<String>,
    pub mode: Option<String>,
    pub variant: Option<String>,
}

/// `scheme set`. Returns whether to apply the scheme's colours.
pub fn set(ctx: &mut Ctx, a: &SetArgs) -> Result<bool, Stop> {
    ctx.load_scheme()?;
    if a.notify {
        ctx.cur_mut().notify = true;
    }
    // argparse values are only acted on when truthy, so empty strings count as absent
    let nonempty = |o: &Option<String>| o.clone().filter(|s| !s.is_empty());
    let (name, flavour, mode, variant) =
        (nonempty(&a.name), nonempty(&a.flavour), nonempty(&a.mode), nonempty(&a.variant));
    if a.random {
        ctx.set_random()?;
    } else if name.is_some() || flavour.is_some() || mode.is_some() || variant.is_some() {
        if let Some(n) = name {
            ctx.set_name(&n)?;
        }
        if let Some(f) = flavour {
            ctx.set_flavour(&f)?;
        }
        if let Some(m) = mode {
            ctx.set_mode(&m)?;
        }
        if let Some(v) = variant {
            ctx.set_variant(&v)?;
        }
    } else {
        ctx.print("No args given. Use --name, --flavour, --mode, --variant or --random to set a scheme".into());
        return Ok(false);
    }
    Ok(true)
}

/// `scheme get`. Flags are name, flavour, mode, variant.
pub fn get(ctx: &mut Ctx, flags: [bool; 4]) -> Result<(), Stop> {
    ctx.load_scheme()?;
    let s = ctx.cur().clone();
    if flags.iter().any(|&f| f) {
        for (on, v) in flags.into_iter().zip([s.name, s.flavour, s.mode, s.variant]) {
            if on {
                ctx.print(v);
            }
        }
        return Ok(());
    }
    // Scheme.__str__
    let mut lines = Vec::new();
    for (n, c) in &s.colours {
        let c = c.as_str().unwrap_or_default();
        let ch = |a| py_int16(&char_slice(c, a, a + 2)).ok_or(Stop::Fallback);
        lines.push(format!("{n}: \x1b[38;2;{};{};{}m{c}\x1b[0m", ch(0)?, ch(2)?, ch(4)?));
    }
    ctx.print(format!(
        "Current scheme:\n    Name: {}\n    Flavour: {}\n    Mode: {}\n    Variant: {}\n    Colours:\n        {}",
        s.name,
        s.flavour,
        s.mode,
        s.variant,
        lines.join("\n        ")
    ));
    Ok(())
}

/// `scheme list`. Flags are names, flavours, modes, variants.
pub fn list(ctx: &mut Ctx, flags: [bool; 4]) -> Result<(), Stop> {
    let multiple = flags.iter().filter(|&&f| f).count() > 1;
    if flags.iter().any(|&f| f) {
        for (i, label) in ["Names:", "Flavours:", "Modes:", "Variants:"].into_iter().enumerate() {
            if !flags[i] {
                continue;
            }
            let items = match i {
                0 => ctx.names()?,
                1 => {
                    ctx.load_scheme()?;
                    ctx.flavours(&ctx.cur().name)?
                }
                2 => {
                    ctx.load_scheme()?;
                    ctx.modes(&ctx.cur().name, &ctx.cur().flavour)?
                }
                _ => VARIANTS.iter().map(|v| v.to_string()).collect(),
            };
            let line = if multiple {
                std::iter::once(label.to_owned()).chain(items).collect::<Vec<_>>().join(" ")
            } else {
                items.join("\n")
            };
            ctx.print(line);
        }
        return Ok(());
    }

    ctx.load_scheme()?;
    let cur = ctx.cur().clone();
    let mut schemes = Map::new();
    for name in ctx.names()? {
        let mut flavours = Map::new();
        for flavour in ctx.flavours(&name)? {
            let colours = if name == "dynamic" {
                // get_colours_for_image() reads the *global* scheme, so every dynamic flavour is the same
                ctx.image_colours(&ctx.thumb, &cur.variant, &cur.flavour, &cur.mode)
            } else {
                let modes = ctx.modes(&name, &flavour)?;
                let mode = if modes.contains(&cur.mode) { &cur.mode } else { modes.first().ok_or(Stop::Fallback)? };
                read_colours(&ctx.colours_path(&name, &flavour, mode))
            };
            match colours {
                Ok(c) => {
                    flavours.insert(flavour, Value::Object(c));
                }
                Err(Stop::Fallback) => return Err(Stop::Fallback),
                Err(_) => {} // ValueError: skipped
            }
        }
        schemes.insert(name, Value::Object(flavours));
    }
    ctx.print(dumps(&Value::Object(schemes)));
    Ok(())
}
