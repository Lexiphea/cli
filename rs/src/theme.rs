//! App themes: `utils/theme.py` and `utils/colour.py`.

use crate::scheme::{Scheme, Stop};
use crate::util::{
    Colours, Paths, atomic_write, char_slice, dumps, file_name, is_lua_config, make_temp_dir, out, py_int16,
    read_text, truthy, which,
};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

const DISCORD_CLIENTS: [&str; 6] = ["Equicord", "Vencord", "BetterDiscord", "equibop", "vesktop", "legcord"];
const SEQUENCE_KEYS: [&str; 21] = [
    "onSurface", "surface", "secondary", "term0", "term1", "term2", "term3", "term4", "term5", "term6", "term7",
    "term8", "term9", "term10", "term11", "term12", "term13", "term14", "term15", "primary", "tertiary",
];

/// `cli.json`, checked up front so the apply step can't hit a shape Python would crash on.
#[derive(Default)]
pub struct Config {
    /// Invalid JSON: Python warns (on stdout) each time it reads the config, and uses `{}`.
    pub invalid: bool,
    root: Map<String, Value>,
}

const WARN_INVALID: &str = "\x1b[33m:: Warning: failed to parse config, invalid JSON\x1b[0m";

impl Config {
    pub fn load(p: &Paths) -> Result<Config, Stop> {
        let bytes = match fs::read(p.c_config.join("cli.json")) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(_) => return Err(Stop::Fallback),
        };
        let text = String::from_utf8(bytes).map_err(|_| Stop::Fallback)?;
        match serde_json::from_str(&text) {
            Ok(Value::Object(root)) => Ok(Config { invalid: false, root }),
            Ok(_) => Err(Stop::Fallback),
            Err(_) => Ok(Config { invalid: true, root: Map::new() }),
        }
    }

    /// `get_config().get(key, {})`, which must be a dict.
    pub fn section(&self, key: &str) -> Result<Map<String, Value>, Stop> {
        match self.root.get(key) {
            None => Ok(Map::new()),
            Some(Value::Object(m)) => Ok(m.clone()),
            Some(_) => Err(Stop::Fallback),
        }
    }

    /// `get_config()` as called from the wallpaper post-hook step (warns again if invalid).
    pub fn reread_warn(&self) {
        if self.invalid {
            out(WARN_INVALID);
        }
    }
}

/// A truthy `postHook` must be a string.
pub fn post_hook(section: &Map<String, Value>) -> Result<Option<String>, Stop> {
    match section.get("postHook") {
        Some(v) if truthy(v) => v.as_str().map(|s| Some(s.to_owned())).ok_or(Stop::Fallback),
        _ => Ok(None),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c.flat_map(char::to_lowercase)).collect())
}

/// `cfg.get(f"iconTheme{Mode}") or cfg.get("iconTheme")`
fn icon_theme(cfg: &Map<String, Value>, mode: &str) -> Result<Option<String>, Stop> {
    let v = match cfg.get(&format!("iconTheme{}", capitalize(mode))) {
        Some(v) if truthy(v) => Some(v),
        _ => cfg.get("iconTheme"),
    };
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(Stop::Fallback),
    }
}

fn discord_clients(cfg: &Map<String, Value>) -> Result<Vec<String>, Stop> {
    match cfg.get("discordClients") {
        None => Ok(DISCORD_CLIENTS.iter().map(|s| s.to_string()).collect()),
        Some(Value::Array(a)) => a.iter().map(|v| v.as_str().map(str::to_owned).ok_or(Stop::Fallback)).collect(),
        Some(_) => Err(Stop::Fallback),
    }
}

fn check(cfg: &Map<String, Value>, key: &str) -> bool {
    cfg.get(key).is_none_or(truthy)
}

/// Everything about the config and colours that would crash `apply_colours` mid-way in Python.
pub fn precheck(cfg: &Config, s: &Scheme) -> Result<(), Stop> {
    let theme = cfg.section("theme")?;
    if check(&theme, "enableTerm") && SEQUENCE_KEYS.iter().any(|k| !s.colours.contains_key(*k)) {
        return Err(Stop::Fallback);
    }
    discord_clients(&theme)?;
    icon_theme(&theme, &s.mode)?;
    post_hook(&theme)?;
    Ok(())
}

fn col<'a>(c: &'a Colours, k: &str) -> &'a str {
    c.get(k).and_then(Value::as_str).unwrap_or_default()
}

fn colours_iter(c: &Colours) -> impl Iterator<Item = (&str, &str)> {
    c.iter().map(|(k, v)| (k.as_str(), v.as_str().unwrap_or_default()))
}

fn gen_conf(c: &Colours) -> String {
    colours_iter(c).map(|(n, v)| format!("${n} = {v}\n")).collect()
}

fn gen_lua(c: &Colours) -> String {
    let body: String = colours_iter(c).map(|(n, v)| format!("  {n} = \"{v}\",\n")).collect();
    format!("return {{\n{body}}}")
}

fn gen_scss(c: &Colours) -> String {
    colours_iter(c).map(|(n, v)| format!("${n}: #{v};\n")).collect()
}

fn gen_replace(c: &Colours, template: &Path, hash: bool) -> io::Result<String> {
    let mut t = read_text(template)?;
    for (n, v) in colours_iter(c) {
        let pat = format!("{{{{ ${n} }}}}");
        if t.contains(&pat) {
            t = t.replace(&pat, &if hash { format!("#{v}") } else { v.to_owned() });
        }
    }
    Ok(t)
}

/// `colour.Colour`
struct Colour {
    hex: [String; 4],
    rgb: [i64; 4],
    hsl: [i64; 3],
}

impl Colour {
    fn new(hex: &str) -> Result<Colour, String> {
        let mut hex = hex.to_owned();
        while hex.chars().count() < 8 {
            hex.push('f');
        }
        let parts = [0, 2, 4, 6].map(|i| char_slice(&hex, i, i + 2));
        let mut rgb = [0; 4];
        for (v, h) in rgb.iter_mut().zip(&parts) {
            *v = py_int16(h).ok_or_else(|| format!("invalid literal for int() with base 16: '{h}'"))?;
        }
        let [r, g, b] = [rgb[0], rgb[1], rgb[2]].map(|v| v as f64 / 255.0);
        let (cmax, cmin) = (r.max(g).max(b), r.min(g).min(b));
        let delta = cmax - cmin;
        let l = (cmax + cmin) / 2.0;
        let s = if delta == 0.0 { 0.0 } else { delta / (1.0 - (2.0 * l - 1.0).abs()) };
        let h = if delta == 0.0 {
            0.0
        } else if cmax == r {
            60.0 * ((g - b) / delta).rem_euclid(6.0)
        } else if cmax == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
        // Python's round() rounds half to even
        let hsl = [h, s * 100.0, l * 100.0].map(|x| x.round_ties_even() as i64);
        Ok(Colour { hex: parts, rgb, hsl })
    }

    fn get(&self, form: &str) -> Option<String> {
        let [r, g, b, a] = self.rgb;
        let [h, s, l] = self.hsl;
        Some(match form {
            "hex" => self.hex[..3].concat(),
            "hexalpha" => self.hex.concat(),
            "rgb" => format!("rgb({r},{g},{b})"),
            "rgbalpha" => format!("rgba({r},{g},{b},{a})"),
            "red" => r.to_string(),
            "green" => g.to_string(),
            "blue" => b.to_string(),
            "hsl" => format!("hsl({h},{s}%,{l}%)"),
            "hue" => h.to_string(),
            "saturation" => s.to_string(),
            "lightness" => l.to_string(),
            _ => return None,
        })
    }
}

/// `re.sub(r"\{\{((?:(?!\{\{|\}\}).)*)\}\}", f, t)`: the field runs to the first `{{`, `}}` or newline.
fn replace_fields(t: &str, f: impl Fn(&str) -> Option<String>) -> String {
    let mut res = String::with_capacity(t.len());
    let mut i = 0;
    while let Some(off) = t[i..].find("{{") {
        let start = i + off;
        res.push_str(&t[i..start]);
        let rest = &t[start + 2..];
        let b = rest.as_bytes();
        let pair = |j: usize, c: u8| b[j] == c && b.get(j + 1) == Some(&c);
        let mut end = 0;
        while end < b.len() && b[end] != b'\n' && !pair(end, b'{') && !pair(end, b'}') {
            end += 1;
        }
        if rest[end..].starts_with("}}") {
            let group = &rest[..end];
            match f(group) {
                Some(v) => res.push_str(&v),
                None => res.push_str(&t[start..start + 2 + end + 2]),
            }
            i = start + 2 + end + 2;
        } else {
            res.push('{');
            i = start + 1;
        }
    }
    res.push_str(&t[i..]);
    res
}

/// `re.sub(r"\{\{\s*mode\s*\}\}", mode, t)`
fn replace_mode(t: &str, mode: &str) -> String {
    let mut res = String::with_capacity(t.len());
    let mut i = 0;
    while let Some(off) = t[i..].find("{{") {
        let start = i + off;
        res.push_str(&t[i..start]);
        let a = t[start + 2..].trim_start();
        let matched = a.strip_prefix("mode").map(str::trim_start).and_then(|b| b.strip_prefix("}}"));
        match matched {
            Some(after) => {
                res.push_str(mode);
                i = t.len() - after.len();
            }
            None => {
                res.push('{');
                i = start + 1;
            }
        }
    }
    res.push_str(&t[i..]);
    res
}

fn gen_replace_dynamic(c: &Colours, template: &Path, mode: &str) -> Result<String, String> {
    let dynamic: HashMap<&str, Colour> =
        colours_iter(c).map(|(n, v)| Colour::new(v).map(|col| (n, col))).collect::<Result<_, _>>()?;
    let t = read_text(template).map_err(|e| e.to_string())?;
    let filled = replace_fields(&t, |group| {
        let data: Vec<&str> = group.trim().split('.').collect();
        let [name, form] = data[..] else { return None };
        dynamic.get(name)?.get(form)
    });
    Ok(replace_mode(&filled, mode))
}

fn hex_to_ansi(c: &str, i: &[u8]) -> String {
    let i: Vec<String> = i.iter().map(u8::to_string).collect();
    format!(
        "\x1b]{};rgb:{}/{}/{}\x1b\\",
        i.join(";"),
        char_slice(c, 0, 2),
        char_slice(c, 2, 4),
        char_slice(c, 4, 6)
    )
}

fn gen_sequences(c: &Colours) -> String {
    let mut s = hex_to_ansi(col(c, "onSurface"), &[10])
        + &hex_to_ansi(col(c, "surface"), &[11])
        + &hex_to_ansi(col(c, "secondary"), &[12])
        + &hex_to_ansi(col(c, "secondary"), &[17]);
    for n in 0..16u8 {
        s += &hex_to_ansi(col(c, &format!("term{n}")), &[4, n]);
    }
    s + &hex_to_ansi(col(c, "primary"), &[4, 16])
        + &hex_to_ansi(col(c, "secondary"), &[4, 17])
        + &hex_to_ansi(col(c, "tertiary"), &[4, 18])
}

type Res = Result<(), String>;

fn e2s(e: io::Error) -> String {
    e.to_string()
}

fn write(path: PathBuf, content: &str) -> Res {
    atomic_write(&path, content).map_err(e2s)
}

/// `subprocess.run(cmd, stderr=DEVNULL)`, result ignored
fn run_quiet(cmd: &[&str]) -> Res {
    Command::new(cmd[0]).args(&cmd[1..]).stderr(Stdio::null()).status().map_err(e2s)?;
    Ok(())
}

fn apply_terms(p: &Paths, seq: &str) -> Res {
    let state = p.c_state.join("sequences.txt");
    fs::create_dir_all(&p.c_state).map_err(e2s)?;
    fs::write(state, seq).map_err(e2s)?;
    const O_NONBLOCK: i32 = 0o4000;
    const O_NOCTTY: i32 = 0o400;
    for e in fs::read_dir("/dev/pts").map_err(e2s)?.flatten() {
        let pt = e.path();
        if file_name(&pt).is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
            // Busy, closed or inaccessible terminals are skipped
            if let Ok(mut f) = OpenOptions::new().write(true).custom_flags(O_NONBLOCK | O_NOCTTY).open(&pt) {
                let _ = f.write(seq.as_bytes());
            }
        }
    }
    Ok(())
}

fn apply_hypr(p: &Paths, conf: &str) -> Res {
    let ext = if is_lua_config() { "lua" } else { "conf" };
    write(p.config.join(format!("hypr/scheme/current.{ext}")), conf)
}

fn apply_discord(p: &Paths, scss: &str, clients: &[String]) -> Res {
    let tmp = make_temp_dir("w").map_err(e2s)?;
    let conf = run_sass(p, &tmp, scss);
    let _ = fs::remove_dir_all(&tmp);
    let conf = conf?;
    for client in clients {
        write(p.config.join(client).join("themes/caelestia.theme.css"), &conf)?;
    }
    Ok(())
}

/// `subprocess.check_output(["sass", "-I", tmp, discord.scss], text=True)`
fn run_sass(p: &Paths, tmp: &Path, scss: &str) -> Result<String, String> {
    fs::write(tmp.join("_colours.scss"), scss).map_err(e2s)?;
    let o = Command::new("sass")
        .arg("-I")
        .arg(tmp)
        .arg(p.templates().join("discord.scss"))
        .stdout(Stdio::piped())
        .spawn()
        .map_err(e2s)?
        .wait_with_output()
        .map_err(e2s)?;
    if !o.status.success() {
        return Err(format!("sass exited with {}", o.status));
    }
    let s = String::from_utf8(o.stdout).map_err(|e| e.to_string())?;
    Ok(s.replace("\r\n", "\n").replace('\r', "\n"))
}

fn apply_simple(c: &Colours, template: PathBuf, hash: bool, dest: PathBuf, extra: &[(&str, &str)]) -> Res {
    let mut t = gen_replace(c, &template, hash).map_err(e2s)?;
    for (from, to) in extra {
        t = t.replace(from, to);
    }
    write(dest, &t)
}

fn apply_gtk(p: &Paths, c: &Colours, mode: &str, icon: Option<&str>) -> Res {
    let gtk = gen_replace(c, &p.templates().join("gtk.css"), true).map_err(e2s)?;
    let thunar = gen_replace(c, &p.templates().join("thunar.css"), true).map_err(e2s)?;
    for v in ["gtk-3.0", "gtk-4.0"] {
        write(p.config.join(v).join("gtk.css"), &gtk)?;
        write(p.config.join(v).join("thunar.css"), &thunar)?;
    }
    let icon = icon.map_or_else(|| format!("Papirus-{}", capitalize(mode)), str::to_owned);
    let input = format!("[/]\ngtk-theme='adw-gtk3-dark'\ncolor-scheme='prefer-{mode}'\nicon-theme='{icon}'\n");
    let mut child = Command::new("dconf").args(["load", "/org/gnome/desktop/interface/"]).stdin(Stdio::piped()).spawn().map_err(e2s)?;
    let _ = child.stdin.take().expect("piped").write_all(input.as_bytes());
    child.wait().map_err(e2s)?;
    sync_papirus(p, col(c, "primary"))
}

fn sync_papirus(p: &Paths, hex: &str) -> Res {
    if !which("papirus-folders") {
        return Ok(());
    }
    let dirs = [
        PathBuf::from("/usr/share/icons/Papirus"),
        PathBuf::from("/usr/share/icons/Papirus-Dark"),
        p.home.join(".local/share/icons/Papirus"),
        p.home.join(".icons/Papirus"),
    ];
    if !dirs.iter().any(|d| d.exists()) {
        return Ok(());
    }
    let ch = |a| py_int16(&char_slice(hex, a, a + 2)).ok_or_else(|| format!("invalid hex colour '{hex}'"));
    let (r, g, b) = (ch(0)?, ch(2)?, ch(4)?);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let saturation = if max == 0 { 0 } else { (max - min) * 100 / max };
    let color = if saturation < 20 {
        if max < 85 {
            "black"
        } else if max < 170 {
            "grey"
        } else {
            "white"
        }
    } else {
        hue_colour(r, g, b, max, saturation < 60 && max > 180)
    };
    let mut cmd = Command::new("sudo");
    cmd.args(["-n", "papirus-folders", "-C", color, "-u"]).stdout(Stdio::null()).stderr(Stdio::null());
    // start_new_session=True, and not waited on
    unsafe extern "C" {
        fn setsid() -> i32;
    }
    // SAFETY: setsid() is async-signal-safe and touches no Rust state between fork and exec
    unsafe {
        cmd.pre_exec(|| {
            setsid();
            Ok(())
        });
    }
    let _ = cmd.spawn();
    Ok(())
}

/// `_determine_hue_color`. Python `//` on these non-negative ints is plain integer division.
fn hue_colour(r: i64, g: i64, b: i64, brightness: i64, pale: bool) -> &'static str {
    if b > r && b > g {
        let r_ratio = if b > 0 { r * 100 / b } else { 0 };
        let g_ratio = if b > 0 { g * 100 / b } else { 0 };
        if r_ratio > 70 && g_ratio > 70 {
            if (r - g).abs() < 15 {
                "blue"
            } else if r > g {
                "violet"
            } else {
                "cyan"
            }
        } else if r_ratio > 60 && r > g {
            "violet"
        } else if g_ratio > 60 && g > r {
            "cyan"
        } else {
            "blue"
        }
    } else if r > g && r > b {
        if g > b + 30 {
            let rg_ratio = if r > 0 { g * 100 / r } else { 0 };
            match (pale, rg_ratio > 70) {
                (true, true) if brightness < 220 => "palebrown",
                (true, _) => "paleorange",
                (false, true) if brightness < 180 => "brown",
                (false, _) => "orange",
            }
        } else if b > g + 20 || pale {
            "pink"
        } else {
            "red"
        }
    } else if g > r && g > b {
        if r > b + 30 { "yellow" } else { "green" }
    } else {
        "grey"
    }
}

fn apply_qt(p: &Paths, c: &Colours, mode: &str, icon: Option<&str>) -> Res {
    let colours = gen_replace(c, &p.templates().join(format!("qt{mode}.colors")), true).map_err(e2s)?;
    write(p.config.join("qtengine/caelestia.colors"), &colours)?;
    let m = capitalize(mode);
    let mut config = read_text(&p.templates().join("qtengine.json")).map_err(e2s)?.replace("{{ $mode }}", &m);
    if let Some(icon) = icon {
        config = config.replace(&format!("\"iconTheme\": \"Papirus-{m}\""), &format!("\"iconTheme\": \"{icon}\""));
    }
    write(p.config.join("qtengine/config.json"), &config)
}

fn apply_chromium(c: &Colours) -> Res {
    let surface = c.get("surface").and_then(Value::as_str).ok_or("'surface'")?;
    let mut policy = Map::new();
    policy.insert("BrowserThemeColor".into(), format!("#{surface}").into());
    policy.insert("BrowserColorScheme".into(), "device".into());
    let policy = dumps(&Value::Object(policy));
    for (cmd, dir) in [
        ("chromium", "/etc/chromium/policies/managed"),
        ("brave", "/etc/brave/policies/managed"),
        ("google-chrome-stable", "/etc/opt/chrome/policies/managed"),
    ] {
        if !which(cmd) {
            continue;
        }
        let dir = Path::new(dir);
        if !dir.is_dir() {
            run_quiet(&["sudo", "-n", "mkdir", "-p", &dir.to_string_lossy()])?;
        }
        if !dir.is_dir() {
            out(&format!("Unable to create {} directory", dir.display()));
            continue;
        }
        let mut child = Command::new("sudo")
            .args(["-n", "tee"])
            .arg(dir.join("caelestia.json"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(e2s)?;
        let _ = child.stdin.take().expect("piped").write_all(policy.as_bytes());
        child.wait().map_err(e2s)?;
        Command::new(cmd)
            .args(["--refresh-platform-policy", "--no-startup-window"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(e2s)?;
    }
    Ok(())
}

fn apply_zed(p: &Paths, c: &Colours, mode: &str) -> Res {
    let path = p.config.join("zed/themes/caelestia.json");
    // Zed's file watcher doesn't follow symlinks, so replace one with a regular file
    if path.is_symlink() {
        fs::remove_file(&path).map_err(e2s)?;
    }
    let content = gen_replace_dynamic(c, &p.templates().join("zed.json"), mode)?;
    write(path, &content)
}

fn apply_user_templates(p: &Paths, c: &Colours, mode: &str) -> Res {
    let dir = p.c_config.join("templates");
    if !dir.is_dir() {
        return Ok(());
    }
    for e in fs::read_dir(&dir).map_err(e2s)? {
        let file = e.map_err(e2s)?.path();
        if file.is_file() {
            let content = gen_replace_dynamic(c, &file, mode)?;
            write(p.c_state.join("theme").join(file.file_name().expect("dir entry")), &content)?;
        }
    }
    Ok(())
}

fn log(name: &str, r: Res) {
    if let Err(e) = r {
        eprintln!("\x1b[31m:: Error: exception during \"{name}()\": {e}\x1b[0m");
    }
}

/// `theme.apply_colours`: every enabled app theme in parallel under `theme.lock`, then the post hook.
pub fn apply_colours(p: &Paths, cfg: &Config, s: &Scheme) -> io::Result<()> {
    fs::create_dir_all(&p.c_state)?;
    let lock_path = p.c_state.join("theme.lock");
    let r = (|| -> io::Result<()> {
        let lock = File::create(&lock_path)?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(()),
            Err(TryLockError::Error(e)) => return Err(e),
        }
        if cfg.invalid {
            out(WARN_INVALID);
        }
        // Validated in precheck(), so these can't fail here
        let theme = cfg.section("theme").unwrap_or_default();
        let clients = discord_clients(&theme).unwrap_or_default();
        let icon = icon_theme(&theme, &s.mode).unwrap_or_default();
        let (c, mode, t) = (&s.colours, s.mode.as_str(), p.templates());
        let on = |k: &str| check(&theme, k);
        let warp = if mode == "dark" { "darker" } else { "lighter" };
        thread::scope(|sc| {
            if on("enableTerm") {
                let seq = gen_sequences(c);
                sc.spawn(move || log("apply_terms", apply_terms(p, &seq)));
            }
            if on("enableHypr") {
                let conf = if is_lua_config() { gen_lua(c) } else { gen_conf(c) };
                sc.spawn(move || log("apply_hypr", apply_hypr(p, &conf)));
            }
            if on("enableDiscord") {
                let (scss, clients) = (gen_scss(c), &clients);
                sc.spawn(move || log("apply_discord", apply_discord(p, &scss, clients)));
            }
            // (config key, fn name, template, add '#', destination, extra replacements, app to signal)
            let simple = [
                ("enableSpicetify", "apply_spicetify", format!("spicetify-{mode}.ini"), false,
                    p.config.join("spicetify/Themes/caelestia/color.ini"), vec![], None),
                ("enablePandora", "apply_pandora", "pandora.json".into(), true,
                    p.data.join("PandoraLauncher/themes/caelestia.json"), vec![("{{ $mode }}", mode)], None),
                ("enableFuzzel", "apply_fuzzel", "fuzzel.ini".into(), false, p.config.join("fuzzel/fuzzel.ini"), vec![], None),
                ("enableBtop", "apply_btop", "btop.theme".into(), true,
                    p.config.join("btop/themes/caelestia.theme"), vec![], Some("btop")),
                ("enableNvtop", "apply_nvtop", "nvtop.colors".into(), true, p.config.join("nvtop/nvtop.colors"), vec![], None),
                ("enableHtop", "apply_htop", "htop.theme".into(), true, p.config.join("htop/htoprc"), vec![], Some("htop")),
                ("enableWarp", "apply_warp", "warp.yaml".into(), true,
                    p.data.join("warp-terminal/themes/caelestia.yaml"), vec![("{{ $warp_mode }}", warp)], None),
                ("enableCava", "apply_cava", "cava.conf".into(), true, p.config.join("cava/config"), vec![], Some("cava")),
            ];
            for (key, name, template, hash, dest, extra, signal) in simple {
                if on(key) {
                    let template = t.join(template);
                    sc.spawn(move || {
                        let mut r = apply_simple(c, template, hash, dest, &extra);
                        if let (Ok(()), Some(app)) = (&r, signal) {
                            r = run_quiet(&["killall", "-USR2", app]);
                        }
                        log(name, r);
                    });
                }
            }
            let icon = icon.as_deref();
            if on("enableGtk") {
                sc.spawn(move || log("apply_gtk", apply_gtk(p, c, mode, icon)));
            }
            if on("enableQt") {
                sc.spawn(move || log("apply_qt", apply_qt(p, c, mode, icon)));
            }
            if on("enableChromium") {
                sc.spawn(move || log("apply_chromium", apply_chromium(c)));
            }
            if on("enableZed") {
                sc.spawn(move || log("apply_zed", apply_zed(p, c, mode)));
            }
            sc.spawn(move || log("apply_user_templates", apply_user_templates(p, c, mode)));
        });

        if let Ok(Some(hook)) = post_hook(&theme) {
            Command::new("/bin/sh")
                .arg("-c")
                .arg(hook)
                .envs(scheme_env(s))
                .stderr(Stdio::null())
                .status()?;
        }
        Ok(())
    })();
    let _ = fs::remove_file(&lock_path);
    r
}

/// The `SCHEME_*` variables post hooks get.
pub fn scheme_env(s: &Scheme) -> [(&'static str, String); 5] {
    [
        ("SCHEME_NAME", s.name.clone()),
        ("SCHEME_FLAVOUR", s.flavour.clone()),
        ("SCHEME_MODE", s.mode.clone()),
        ("SCHEME_VARIANT", s.variant.clone()),
        ("SCHEME_COLOURS", dumps(&Value::Object(s.colours.clone()))),
    ]
}
