//! Paths, buffered stdout, Python-compatible JSON output, files and small process helpers.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::hash_map::RandomState;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::hash::BuildHasher;
use std::io::{self, IsTerminal, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

/// A colour scheme's colours: name -> hex, in file order. Values are always strings.
pub type Colours = Map<String, Value>;

/// The locations `utils/paths.py` resolves from the environment.
pub struct Paths {
    pub home: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub c_config: PathBuf,
    pub c_state: PathBuf,
    pub c_cache: PathBuf,
    pub cli_data: PathBuf,
    pub wallpapers: PathBuf,
}

impl Paths {
    pub fn from_env() -> Self {
        let home = PathBuf::from(env::var_os("HOME").unwrap_or_default());
        // Collecting components drops `//`, `/./` and trailing slashes, as pathlib does
        let dir = |var: &str, default: &str| -> PathBuf {
            env::var_os(var).map_or_else(|| home.join(default), |v| Path::new(&v).components().collect())
        };
        let config = dir("XDG_CONFIG_HOME", ".config");
        let data = dir("XDG_DATA_HOME", ".local/share");
        let state = dir("XDG_STATE_HOME", ".local/state");
        let cache = dir("XDG_CACHE_HOME", ".cache");
        let pictures = dir("XDG_PICTURES_DIR", "Pictures");
        let wallpapers = env::var_os("CAELESTIA_WALLPAPERS_DIR")
            .map_or_else(|| pictures.join("Wallpapers"), |v| Path::new(&v).components().collect());
        // The Python package's own data dir, so templates, schemes and the sass argv are identical
        let cli_data = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("src/caelestia/data");
        Paths {
            c_config: config.join("caelestia"),
            c_state: state.join("caelestia"),
            c_cache: cache.join("caelestia"),
            home,
            config,
            data,
            cli_data,
            wallpapers,
        }
    }

    pub fn scheme_path(&self) -> PathBuf {
        self.c_state.join("scheme.json")
    }

    pub fn wall_dir(&self) -> PathBuf {
        self.c_state.join("wallpaper")
    }

    pub fn templates(&self) -> PathBuf {
        self.cli_data.join("templates")
    }
}

// Python block-buffers stdout when it isn't a terminal, so child processes (post hooks) print
// before anything the CLI printed. Mirror that: buffer until exit unless stdout is a terminal.
static OUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

fn stdout_is_tty() -> bool {
    static TTY: OnceLock<bool> = OnceLock::new();
    *TTY.get_or_init(|| io::stdout().is_terminal())
}

/// `print(s)`.
pub fn out(s: &str) {
    if stdout_is_tty() {
        let mut o = io::stdout().lock();
        let _ = writeln!(o, "{s}");
        let _ = o.flush();
    } else {
        let mut b = OUT.lock().unwrap();
        b.extend_from_slice(s.as_bytes());
        b.push(b'\n');
    }
}

pub fn flush_out() {
    let b = OUT.lock().unwrap();
    let mut o = io::stdout().lock();
    let _ = o.write_all(&b);
    let _ = o.flush();
}

/// `json.dumps(v)`: `", "` and `": "` separators, `ensure_ascii`, insertion order.
pub fn dumps(v: &Value) -> String {
    let mut s = String::new();
    dump(v, &mut s);
    s
}

fn dump(v: &Value, s: &mut String) {
    match v {
        Value::Null => s.push_str("null"),
        Value::Bool(b) => s.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => s.push_str(&n.to_string()),
        Value::String(x) => dump_str(x, s),
        Value::Array(a) => {
            s.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                dump(x, s);
            }
            s.push(']');
        }
        Value::Object(m) => {
            s.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                dump_str(k, s);
                s.push_str(": ");
                dump(x, s);
            }
            s.push('}');
        }
    }
}

fn dump_str(x: &str, s: &mut String) {
    s.push('"');
    for c in x.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            '\x08' => s.push_str("\\b"),
            '\x0c' => s.push_str("\\f"),
            ' '..='~' => s.push(c),
            _ => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    s.push_str(&format!("\\u{u:04x}"));
                }
            }
        }
    }
    s.push('"');
}

/// Python's `repr()` of a list of strings, as used in notification bodies.
pub fn py_repr_list(items: &[String]) -> String {
    let parts: Vec<String> = items
        .iter()
        .map(|x| {
            let q = if x.contains('\'') && !x.contains('"') { '"' } else { '\'' };
            let mut s = String::from(q);
            for c in x.chars() {
                match c {
                    '\\' => s.push_str("\\\\"),
                    '\n' => s.push_str("\\n"),
                    '\r' => s.push_str("\\r"),
                    '\t' => s.push_str("\\t"),
                    c if c == q => {
                        s.push('\\');
                        s.push(c);
                    }
                    c if c < ' ' || c == '\x7f' => s.push_str(&format!("\\x{:02x}", c as u32)),
                    c => s.push(c),
                }
            }
            s.push(q);
            s
        })
        .collect();
    format!("[{}]", parts.join(", "))
}

/// `int(s, 16)` for the short slices the CLI parses: surrounding whitespace and a sign are allowed.
pub fn py_int16(s: &str) -> Option<i64> {
    let t = s.trim();
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = i64::from_str_radix(digits, 16).ok()?;
    Some(if neg { -v } else { v })
}

/// Python `s[a:b]` on characters.
pub fn char_slice(s: &str, a: usize, b: usize) -> String {
    s.chars().skip(a).take(b.saturating_sub(a)).collect()
}

/// `Path.read_text()`: UTF-8 with universal newlines.
pub fn read_text(p: &Path) -> io::Result<String> {
    let s = String::from_utf8(fs::read(p)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(if s.contains('\r') { s.replace("\r\n", "\n").replace('\r', "\n") } else { s })
}

/// A random u64 from std's per-process random hasher keys; no RNG crate needed.
pub fn rand_u64() -> u64 {
    RandomState::new().hash_one(0u8)
}

pub fn rand_below(n: usize) -> usize {
    (rand_u64() % n as u64) as usize
}

/// `tempfile`'s 8-character random names.
fn rand_name() -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789_";
    let mut x = rand_u64();
    (0..8)
        .map(|_| {
            let c = CHARS[(x % CHARS.len() as u64) as usize] as char;
            x /= CHARS.len() as u64;
            c
        })
        .collect()
}

/// `paths.atomic_write`: a 0600 `tmpXXXXXXXX` file in the same dir, renamed over `path`.
pub fn atomic_write(path: &Path, content: &str) -> io::Result<()> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    fs::create_dir_all(dir)?;
    let (tmp, mut f) = loop {
        let tmp = dir.join(format!("tmp{}", rand_name()));
        match OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp) {
            Ok(f) => break (tmp, f),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let r = f.write_all(content.as_bytes()).and_then(|_| fs::rename(&tmp, path));
    if r.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    r
}

/// `tempfile.mkdtemp(suffix)` in `$TMPDIR` (or /tmp).
pub fn make_temp_dir(suffix: &str) -> io::Result<PathBuf> {
    let base = env::var_os("TMPDIR").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    loop {
        let d = base.join(format!("tmp{}{suffix}", rand_name()));
        match fs::DirBuilder::new().mode(0o700).create(&d) {
            Ok(()) => return Ok(d),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

use std::os::unix::fs::DirBuilderExt;

/// `paths.compute_hash`: sha256 hex of a file's contents.
pub fn hash_file(p: &Path) -> io::Result<String> {
    let mut f = File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 18];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// `shutil.which(cmd) is not None`.
pub fn which(cmd: &str) -> bool {
    let path = env::var_os("PATH").unwrap_or_else(|| "/bin:/usr/bin".into());
    env::split_paths(&path)
        .any(|d| fs::metadata(d.join(cmd)).is_ok_and(|m| !m.is_dir() && m.permissions().mode() & 0o111 != 0))
}

/// `notify.notify(*args)`: `notify-send -a caelestia-cli ...`, waiting for it and failing on error.
pub fn notify(args: &[String]) -> io::Result<()> {
    let o = Command::new("notify-send")
        .arg("-a")
        .arg("caelestia-cli")
        .args(args)
        .stdout(Stdio::piped())
        .spawn()?
        .wait_with_output()?;
    if o.status.success() { Ok(()) } else { Err(io::Error::other(format!("notify-send failed: {}", o.status))) }
}

/// `hypr.message(msg)` with `is_json=True`.
pub fn hypr_json(msg: &str) -> io::Result<Value> {
    let var = |k: &str| env::var(k).unwrap_or_else(|_| "None".into());
    let sock = format!("{}/hypr/{}/.socket.sock", var("XDG_RUNTIME_DIR"), var("HYPRLAND_INSTANCE_SIGNATURE"));
    let mut s = UnixStream::connect(sock)?;
    s.write_all(format!("j/{msg}").as_bytes())?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf)?;
    serde_json::from_slice(&buf).map_err(io::Error::other)
}

/// `hypr.is_lua_config()`, cached; any failure means "not Lua".
pub fn is_lua_config() -> bool {
    static LUA: OnceLock<bool> = OnceLock::new();
    *LUA.get_or_init(|| {
        hypr_json("status").is_ok_and(|v| v.get("configProvider").and_then(Value::as_str) == Some("lua"))
    })
}

/// Python truthiness of a JSON value.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
    }
}

/// UTF-8 file name of a path, as Python would give it (non-UTF-8 names aren't supported).
pub fn file_name(p: &Path) -> Option<&str> {
    p.file_name().and_then(OsStr::to_str)
}
