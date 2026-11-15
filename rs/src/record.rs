//! `caelestia record`: start, pause or stop a gpu-screen-recorder recording.

use crate::scheme::{Stop, fail};
use crate::theme::Config;
use crate::util::{Paths, hypr_json, new_session, notify};
use serde_json::Value;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const RECORDER: &str = "gpu-screen-recorder";
/// Recording at a high refresh rate (e.g. 280 Hz) makes huge files for no visible gain
const MAX_FPS: i64 = 60;

pub struct Args {
    /// `-r` alone means "pick one with slurp"
    pub region: Option<Option<String>>,
    pub sound: bool,
    pub pause: bool,
    pub clipboard: bool,
}

fn quiet(cmd: &mut Command) -> bool {
    cmd.stdout(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn running() -> bool {
    quiet(Command::new("pidof").arg(RECORDER))
}

pub fn run(p: &Paths, a: &Args) -> Result<(), Stop> {
    let state = p.c_state.join("record");
    if a.pause {
        quiet(Command::new("pkill").args(["-USR2", "-f", RECORDER]));
        Ok(())
    } else if running() {
        stop(p, a, &state).map_err(|e| fail(e.to_string()))
    } else {
        start(p, a, &state)
    }
}

/// `WxH+X+Y`, as slurp prints it.
fn parse_region(r: &str) -> Option<[i64; 4]> {
    let (w, rest) = r.split_once('x')?;
    let (h, rest) = rest.split_once('+')?;
    let (x, y) = rest.split_once('+')?;
    let y = y.split(|c: char| !c.is_ascii_digit() && c != '-').next()?;
    Some([x.parse().ok()?, y.parse().ok()?, w.parse().ok()?, h.parse().ok()?])
}

fn refresh(m: &Value) -> i64 {
    m["refreshRate"].as_f64().unwrap_or(0.0).round_ties_even() as i64
}

fn start(p: &Paths, a: &Args, state: &Path) -> Result<(), Stop> {
    let mut args: Vec<String> = vec!["-w".into()];
    let monitors = hypr_json("monitors").map_err(|e| fail(format!("cannot query Hyprland monitors: {e}")))?;
    let monitors = monitors.as_array().map(Vec::as_slice).unwrap_or_default();
    let int = |m: &Value, k: &str| m[k].as_i64().unwrap_or(0);
    if let Some(region) = &a.region {
        let region = match region {
            Some(r) => r.trim().to_owned(),
            None => {
                let o = Command::new("slurp")
                    .args(["-f", "%wx%h+%x+%y"])
                    .stdin(Stdio::null())
                    .output()
                    .map_err(|e| fail(format!("cannot run slurp: {e}")))?;
                if !o.status.success() {
                    return Err(fail("no region selected"));
                }
                String::from_utf8_lossy(&o.stdout).trim().to_owned()
            }
        };
        let [x, y, w, h] = parse_region(&region).ok_or_else(|| fail(format!("invalid region: {region}")))?;
        // The highest refresh rate of the monitors the region touches
        let fps = monitors
            .iter()
            .filter(|m| {
                let [mx, my, mw, mh] = [int(m, "x"), int(m, "y"), int(m, "width"), int(m, "height")];
                mx < x + w && mx + mw > x && my < y + h && my + mh > y
            })
            .map(refresh)
            .max()
            .unwrap_or(0);
        args.extend(["region".into(), "-region".into(), region, "-f".into(), fps.min(MAX_FPS).to_string()]);
    } else {
        let m = monitors.iter().find(|m| m["focused"] == true).ok_or_else(|| fail("no focused monitor"))?;
        let name = m["name"].as_str().unwrap_or_default().to_owned();
        args.extend([name, "-f".into(), refresh(m).min(MAX_FPS).to_string()]);
    }
    if a.sound {
        args.extend(["-a".into(), "default_output".into()]);
    }
    match Config::load(p)?.section("record")?.get("extraArgs") {
        None => {}
        Some(Value::Array(extra)) if extra.iter().all(Value::is_string) => {
            args.extend(extra.iter().filter_map(Value::as_str).map(str::to_owned));
        }
        Some(_) => return Err(fail("cli.json: record.extraArgs must be a list of strings")),
    }

    let io_fail = |e: io::Error| fail(e.to_string());
    let recording = state.join("recording.mp4");
    fs::create_dir_all(state).map_err(io_fail)?;
    args.extend(["-o".into(), recording.to_string_lossy().into_owned()]);
    let mut child = new_session(Command::new(RECORDER).args(&args))
        .spawn()
        .map_err(|e| fail(format!("cannot run {RECORDER}: {e}")))?;

    let id = notify(&["-p".into(), "Recording started".into(), "Recording...".into()]).map_err(io_fail)?;
    fs::write(state.join("notifid.txt"), &id).map_err(io_fail)?;

    // Report a recorder that dies straight away; one still running after a second is fine
    let end = Instant::now() + Duration::from_secs(1);
    while Instant::now() < end {
        match child.try_wait().map_err(io_fail)? {
            Some(s) if !s.success() => {
                close_notification(&id);
                let code = s.code().map_or_else(|| s.to_string(), |c| c.to_string());
                let msg = format!(
                    "An error occurred attempting to start recorder. Command `{RECORDER} {}` failed with exit code {code}",
                    args.join(" ")
                );
                notify(&["Recording failed".into(), msg]).map_err(io_fail)?;
                return Ok(());
            }
            Some(_) => return Ok(()),
            None => sleep(Duration::from_millis(20)),
        }
    }
    Ok(())
}

fn close_notification(id: &str) {
    let _ = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest=org.freedesktop.Notifications",
            "--object-path=/org/freedesktop/Notifications",
            "--method=org.freedesktop.Notifications.CloseNotification",
            id,
        ])
        .stdout(Stdio::null())
        .status();
}

fn recordings_dir(p: &Paths) -> PathBuf {
    let var = |k: &str| env::var_os(k).map(PathBuf::from);
    var("CAELESTIA_RECORDINGS_DIR")
        .unwrap_or_else(|| var("XDG_VIDEOS_DIR").unwrap_or_else(|| p.home.join("Videos")).join("Recordings"))
}

/// `Path.as_uri()`: percent-encode everything but unreserved characters and `/`.
fn file_uri(p: &Path) -> String {
    let mut s = String::from("file://");
    for b in p.as_os_str().as_encoded_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' | b'/' => s.push(*b as char),
            _ => s.push_str(&format!("%{b:02X}")),
        }
    }
    s
}

fn stop(p: &Paths, a: &Args, state: &Path) -> io::Result<()> {
    quiet(Command::new("pkill").args(["-f", RECORDER]));
    // Wait for the recorder to finish writing, or the file is corrupt
    while running() {
        sleep(Duration::from_millis(100));
    }

    let stamp = Command::new("date").arg("+%Y%m%d_%H-%M-%S").output()?;
    let dir = recordings_dir(p);
    let path = dir.join(format!("recording_{}.mp4", String::from_utf8_lossy(&stamp.stdout).trim()));
    fs::create_dir_all(&dir)?;
    let recording = state.join("recording.mp4");
    if let Err(e) = fs::rename(&recording, &path) {
        if e.kind() != io::ErrorKind::CrossesDevices {
            return Err(e);
        }
        fs::copy(&recording, &path)?;
        fs::remove_file(&recording)?;
    }

    if let Ok(id) = fs::read_to_string(state.join("notifid.txt")) {
        close_notification(&id);
    }

    if a.clipboard {
        let mut c = Command::new("wl-copy").args(["--type", "text/uri-list"]).stdin(Stdio::piped()).spawn()?;
        let uri = file_uri(&fs::canonicalize(&path)?);
        c.stdin.take().expect("piped").write_all(format!("{uri}\n").as_bytes())?;
        c.wait()?;
    }

    let action = notify(&[
        "--action=watch=Watch".into(),
        "--action=open=Open".into(),
        "--action=delete=Delete".into(),
        "Recording stopped".into(),
        format!("Recording saved in {}", path.display()),
    ])?;
    let open = |target: &Path| new_session(Command::new("xdg-open").arg(target)).spawn().map(drop);
    match action.as_str() {
        "watch" => open(&path)?,
        "open" => {
            let shown = Command::new("dbus-send")
                .args([
                    "--session",
                    "--dest=org.freedesktop.FileManager1",
                    "--type=method_call",
                    "/org/freedesktop/FileManager1",
                    "org.freedesktop.FileManager1.ShowItems",
                    &format!("array:string:file://{}", path.display()),
                    "string:",
                ])
                .status()?;
            if !shown.success() {
                open(&dir)?;
            }
        }
        "delete" => fs::remove_file(&path)?,
        _ => {}
    }
    Ok(())
}
