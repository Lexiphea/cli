//! caelestia: the caelestia CLI. `scheme` and `wallpaper`, including every app theme, live here.
//!
//! The Python CLI (`caelestia-py`) still runs every other command, which this binary `exec`s with
//! the same argv, and generates colours, thumbnails and first frames on a cache miss (`_gen`).

mod scheme;
mod theme;
mod util;
mod wallpaper;

use scheme::{Ctx, Effect, SetArgs, Stop, VARIANTS};
use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{self, Command, Stdio};
use theme::Config;
use util::{Paths, flush_out, out, python};

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Flag,
    /// `nargs=None`: needs a value
    Value,
    /// `nargs="?"`: optional value, else the const
    Opt,
}

type Spec = (char, &'static str, Kind);

/// Parsed options by long name. `Some(None)` is a flag or an `Opt` without a value.
type Opts = HashMap<&'static str, Option<String>>;

/// Can argparse take `t` as an option's value? (Not option-like, or a negative number.)
fn is_value(t: &str) -> bool {
    let num = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match t.strip_prefix('-') {
        None | Some("") => true,
        Some(r) => num(r) || r.split_once('.').is_some_and(|(a, b)| (a.is_empty() || num(a)) && num(b)),
    }
}

fn label(spec: &Spec) -> String {
    if spec.0 == '\0' { format!("--{}", spec.1) } else { format!("-{}/--{}", spec.0, spec.1) }
}

/// The subset of argparse these commands use. No abbreviations, `--` or positionals.
fn parse(toks: &[String], specs: &[Spec]) -> Result<Opts, Stop> {
    if toks.iter().any(|t| t == "-h" || t == "--help") {
        return Err(Stop::Help);
    }
    let unknown = |t: &str| Stop::Usage(format!("unrecognized arguments: {t}"));
    let mut opts = Opts::new();
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        i += 1;
        let (mut spec, mut explicit): (&Spec, Option<&str>);
        if let Some(long) = t.strip_prefix("--") {
            let (name, val) = long.split_once('=').map_or((long, None), |(a, b)| (a, Some(b)));
            spec = specs.iter().find(|s| s.1 == name).ok_or_else(|| unknown(t))?;
            explicit = val;
        } else if t.len() >= 2 && t.starts_with('-') && !is_value(t) {
            let mut chars = t[1..].chars();
            let c = chars.next().expect("len >= 2");
            spec = specs.iter().find(|s| s.0 == c).ok_or_else(|| unknown(t))?;
            let rest = chars.as_str();
            explicit = if rest.is_empty() { None } else { Some(rest.strip_prefix('=').unwrap_or(rest)) };
            // Combined flags: `-nN`
            while spec.2 == Kind::Flag && explicit.is_some() {
                if t[1..].contains('=') {
                    return Err(unknown(t));
                }
                opts.insert(spec.1, None);
                let r = explicit.expect("checked");
                let mut chars = r.chars();
                let c = chars.next().expect("non-empty");
                spec = specs.iter().find(|s| s.0 == c).ok_or_else(|| unknown(t))?;
                explicit = Some(chars.as_str()).filter(|s| !s.is_empty());
            }
        } else {
            return Err(unknown(t));
        }
        let next_is_value = toks.get(i).is_some_and(|n| is_value(n));
        let value = match (spec.2, explicit) {
            (Kind::Flag, Some(v)) => {
                return Err(Stop::Usage(format!("argument {}: ignored explicit argument '{v}'", label(spec))));
            }
            (Kind::Flag, None) => None,
            (_, Some(v)) => Some(v.to_owned()),
            (_, None) if next_is_value => {
                i += 1;
                Some(toks[i - 1].clone())
            }
            (Kind::Value, None) => {
                return Err(Stop::Usage(format!("argument {}: expected one argument", label(spec))));
            }
            (_, None) => None,
        };
        opts.insert(spec.1, value);
    }
    Ok(opts)
}

/// Replace this process with the Python CLI, same argv.
fn fallback(argv: &[OsString]) -> ! {
    let py = python();
    let err = Command::new(&py).args(argv).exec();
    eprintln!("caelestia: cannot run {}: {err}", py.display());
    process::exit(127);
}

fn usage_error(msg: String) -> ! {
    eprintln!("caelestia: error: {msg}");
    process::exit(2);
}

fn run_effects(effects: Vec<Effect>) -> io::Result<()> {
    let mkparent = |p: &Path| p.parent().map_or(Ok(()), fs::create_dir_all);
    for e in effects {
        match e {
            Effect::Write(p, s) => util::atomic_write(&p, &s)?,
            Effect::Text(p, s) => {
                mkparent(&p)?;
                fs::write(p, s)?;
            }
            Effect::Link(link, target) => {
                mkparent(&link)?;
                match fs::remove_file(&link) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                    _ => {}
                }
                symlink(target, link)?;
            }
            Effect::Notify(args) => util::notify(&args)?,
            Effect::Print(s) => out(&s),
        }
    }
    Ok(())
}

/// What a plan leaves to do after its effects.
enum Next {
    /// Not ours: run the Python CLI
    Python,
    Done,
    Apply(Config),
    ApplyWall(Config, wallpaper::WallSet),
}

fn flags(o: &Opts, names: [&str; 4]) -> [bool; 4] {
    names.map(|n| o.contains_key(n))
}

fn plan(ctx: &mut Ctx, args: &[String]) -> Result<Next, Stop> {
    use Kind::*;
    let first = args.first().map(String::as_str);
    let second = args.get(1).map(String::as_str);
    let theme_ready = |ctx: &Ctx| -> Result<Config, Stop> {
        let cfg = Config::load(&ctx.p)?;
        theme::precheck(&cfg, ctx.cur())?;
        Ok(cfg)
    };
    match (first, second) {
        (Some("scheme"), Some("list")) => {
            let o = parse(
                &args[2..],
                &[('n', "names", Flag), ('f', "flavours", Flag), ('m', "modes", Flag), ('v', "variants", Flag)],
            )?;
            scheme::list(ctx, flags(&o, ["names", "flavours", "modes", "variants"]))?;
            Ok(Next::Done)
        }
        (Some("scheme"), Some("get")) => {
            let o = parse(
                &args[2..],
                &[('n', "name", Flag), ('f', "flavour", Flag), ('m', "mode", Flag), ('v', "variant", Flag)],
            )?;
            scheme::get(ctx, flags(&o, ["name", "flavour", "mode", "variant"]))?;
            Ok(Next::Done)
        }
        (Some("scheme"), Some("set")) => {
            let mut o = parse(
                &args[2..],
                &[
                    ('\0', "notify", Flag),
                    ('r', "random", Flag),
                    ('n', "name", Value),
                    ('f', "flavour", Value),
                    ('m', "mode", Value),
                    ('v', "variant", Value),
                ],
            )?;
            let names = ctx.names()?;
            let (notify, random) = (o.contains_key("notify"), o.contains_key("random"));
            let mut take = |k: &str| o.remove(k).flatten();
            let a = SetArgs {
                notify,
                random,
                name: take("name"),
                flavour: take("flavour"),
                mode: take("mode"),
                variant: take("variant"),
            };
            let choice = |flag: &str, v: &Option<String>, ok: &[&str]| {
                if let Some(v) = v.as_deref().filter(|v| !ok.contains(v)) {
                    usage_error(format!("argument {flag}: invalid choice: '{v}' (choose from {})", ok.join(", ")));
                }
            };
            choice("-n/--name", &a.name, &names.iter().map(String::as_str).collect::<Vec<_>>());
            choice("-m/--mode", &a.mode, &["dark", "light"]);
            choice("-v/--variant", &a.variant, &VARIANTS);
            ctx.lock()?;
            if scheme::set(ctx, &a)? { Ok(Next::Apply(theme_ready(ctx)?)) } else { Ok(Next::Done) }
        }
        (Some("wallpaper"), _) => {
            let o = parse(
                &args[1..],
                &[
                    ('p', "print", Opt),
                    ('r', "random", Opt),
                    ('f', "file", Value),
                    ('n', "no-filter", Flag),
                    ('t', "threshold", Value),
                    ('N', "no-smart", Flag),
                ],
            )?;
            let no_smart = o.contains_key("no-smart");
            // nargs="?" consts: the current wallpaper (read at parse time) and the wallpapers dir
            let print = match o.get("print") {
                Some(Some(v)) => Some(v.clone()),
                Some(None) => wallpaper::current(ctx)?,
                None => None,
            };
            let random = match o.get("random") {
                Some(Some(v)) => Some(v.clone()),
                Some(None) => Some(ctx.p.wallpapers.to_string_lossy().into_owned()),
                None => None,
            };
            let file = o.get("file").cloned().flatten();
            // Only truthy (non-empty) values count, as in Command.run
            if let Some(p) = print.filter(|s| !s.is_empty()) {
                wallpaper::print_colours(ctx, &p, no_smart)?;
                Ok(Next::Done)
            } else if let Some(f) = file.filter(|s| !s.is_empty()) {
                ctx.lock()?;
                let set = wallpaper::set(ctx, Path::new(&f), no_smart)?;
                Ok(Next::ApplyWall(theme_ready(ctx)?, set))
            } else if let Some(dir) = random.filter(|s| !s.is_empty()) {
                let threshold = match o.get("threshold").cloned().flatten() {
                    Some(t) => t
                        .parse()
                        .map_err(|_| Stop::Usage(format!("argument -t/--threshold: invalid float value: '{t}'")))?,
                    None => 0.8,
                };
                let a =
                    wallpaper::RandomArgs { dir: &dir, no_filter: o.contains_key("no-filter"), threshold, no_smart };
                ctx.lock()?;
                let set = wallpaper::random(ctx, &a)?;
                Ok(Next::ApplyWall(theme_ready(ctx)?, set))
            } else {
                let w = wallpaper::current(ctx)?.filter(|s| !s.is_empty());
                ctx.effects.push(Effect::Print(w.unwrap_or_else(|| "No wallpaper set".into())));
                Ok(Next::Done)
            }
        }
        (Some("scheme"), None) => Err(Stop::Help),
        (Some("scheme"), Some(c)) if c.starts_with('-') => parse(&args[1..], &[]).map(|_| Next::Done),
        (Some("scheme"), Some(c)) => {
            Err(Stop::Usage(format!("argument: invalid choice: '{c}' (choose from list, get, set)")))
        }
        _ => Ok(Next::Python),
    }
}

/// argparse's `-h` output, for the command in `args`.
fn help(args: &[String]) -> &'static str {
    match (args.first().map(String::as_str), args.get(1).map(String::as_str)) {
        (Some("scheme"), Some("list")) => include_str!("help/scheme-list.txt"),
        (Some("scheme"), Some("get")) => include_str!("help/scheme-get.txt"),
        (Some("scheme"), Some("set")) => include_str!("help/scheme-set.txt"),
        (Some("wallpaper"), _) => include_str!("help/wallpaper.txt"),
        _ => include_str!("help/scheme.txt"),
    }
}

/// The wallpaper post hook, after `apply_colours`.
fn wall_hook(ctx: &Ctx, cfg: &Config, set: &wallpaper::WallSet) -> io::Result<()> {
    cfg.reread_warn();
    let Ok(Some(hook)) = cfg.section("wallpaper").and_then(|w| theme::post_hook(&w)) else {
        return Ok(());
    };
    Command::new("/bin/sh")
        .arg("-c")
        .arg(hook)
        .env("WALLPAPER_PATH", &set.wall)
        .envs(theme::scheme_env(ctx.cur()))
        .env("THUMBNAIL_PATH", &set.thumb)
        .stderr(Stdio::null())
        .status()?;
    Ok(())
}

fn main() {
    let argv: Vec<OsString> = env::args_os().skip(1).collect();
    let ours = argv.first().is_some_and(|a| a == "scheme" || a == "wallpaper");
    let Some(args) = argv.iter().map(|a| a.to_str().map(str::to_owned)).collect::<Option<Vec<_>>>() else {
        if ours {
            usage_error("arguments must be UTF-8".into())
        }
        fallback(&argv)
    };
    let mut ctx = Ctx::new(Paths::from_env());
    let next = match plan(&mut ctx, &args) {
        Ok(Next::Python) => fallback(&argv),
        Ok(n) => n,
        Err(Stop::Help) => {
            out(help(&args).trim_end());
            flush_out();
            process::exit(0);
        }
        Err(Stop::Usage(msg)) => usage_error(msg),
        Err(e @ (Stop::Fail(_) | Stop::Missing)) => {
            let msg = match e {
                Stop::Fail(m) => m,
                _ => "file not found".into(),
            };
            let r = run_effects(std::mem::take(&mut ctx.effects));
            flush_out();
            eprintln!("caelestia: {}", r.err().map_or(msg, |e| e.to_string()));
            process::exit(1);
        }
    };
    let r = run_effects(std::mem::take(&mut ctx.effects)).and_then(|()| match &next {
        Next::Python | Next::Done => Ok(()),
        Next::Apply(cfg) => theme::apply_colours(&ctx.p, cfg, ctx.cur()),
        Next::ApplyWall(cfg, set) => {
            theme::apply_colours(&ctx.p, cfg, ctx.cur())?;
            wall_hook(&ctx, cfg, set)
        }
    });
    flush_out();
    if let Err(e) = r {
        eprintln!("caelestia: {e}");
        process::exit(1);
    }
}
