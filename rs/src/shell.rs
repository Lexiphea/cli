//! `caelestia shell`: start, stop or message the Quickshell config through `qs`.

use crate::scheme::{Stop, fail};
use crate::util::{Paths, warn};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub struct Args {
    pub message: Vec<String>,
    pub daemon: bool,
    pub restart: bool,
    pub show: bool,
    pub log: bool,
    pub kill: bool,
    pub log_rules: Option<String>,
}

fn qs() -> Command {
    let mut c = Command::new("qs");
    c.args(["-c", "caelestia"]);
    c
}

/// Replace this process with `qs -c caelestia <args>`.
fn exec_qs(args: &[&str]) -> Stop {
    fail(format!("cannot run qs: {}", qs().args(args).exec()))
}

pub fn run(p: &Paths, a: &Args) -> Result<(), Stop> {
    // The shell logs a warning for every wallpaper thumbnail it hasn't cached yet
    let noise = format!("Cannot open: file://{}/imagecache/", p.c_cache.display());
    let rules = a.log_rules.as_deref();
    if a.show {
        Err(exec_qs(&["ipc", "show"]))
    } else if a.log {
        let mut args = vec!["log"];
        args.extend(rules.map(|r| ["-r", r]).into_iter().flatten());
        print_filtered(qs().args(args), &noise)
    } else if a.kill {
        stop_instances()
    } else if a.restart {
        // Wait for the old instances to exit first, otherwise `-n` silently skips the relaunch
        stop_instances()?;
        start(a, &noise)
    } else if !a.message.is_empty() {
        let mut args = vec!["ipc", "call"];
        args.extend(a.message.iter().map(String::as_str));
        Err(exec_qs(&args))
    } else {
        start(a, &noise)
    }
}

fn start(a: &Args, noise: &str) -> Result<(), Stop> {
    let mut args = vec!["-n"];
    args.extend(a.log_rules.as_deref().map(|r| ["--log-rules", r]).into_iter().flatten());
    if a.daemon {
        args.push("-d");
        return Err(exec_qs(&args));
    }
    print_filtered(qs().args(args), noise)
}

/// Run `cmd`, printing its stdout lines except the noisy ones.
fn print_filtered(cmd: &mut Command, noise: &str) -> Result<(), Stop> {
    let mut child = cmd.stdout(Stdio::piped()).spawn().map_err(|e| fail(format!("cannot run qs: {e}")))?;
    let mut out = std::io::stdout().lock();
    for line in BufReader::new(child.stdout.take().expect("piped")).lines().map_while(Result::ok) {
        if !line.contains(noise) {
            let _ = writeln!(out, "{line}");
        }
    }
    let status = child.wait().map_err(|e| fail(e.to_string()))?;
    if status.success() { Ok(()) } else { Err(fail(format!("qs exited with {status}"))) }
}

fn instance_pids() -> Result<Vec<u64>, Stop> {
    let o = qs().args(["list", "-j"]).output().map_err(|e| fail(format!("cannot run qs: {e}")))?;
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_owned();
    if !o.status.success() {
        let err = if o.stderr.is_empty() { &o.stdout } else { &o.stderr };
        return Err(fail(format!("failed to list shell instances: {}", text(err))));
    }
    // `qs list` exits 0 with a plain text notice instead of JSON when there are no instances
    let out = text(&o.stdout);
    if out.starts_with("No running instances") {
        return Ok(vec![]);
    }
    let list: Vec<Value> =
        serde_json::from_str(&out).map_err(|_| fail(format!("failed to parse shell instance list: {out}")))?;
    Ok(list.iter().filter_map(|i| i["pid"].as_u64()).collect())
}

fn wait_for_exit(timeout: Duration) -> Result<bool, Stop> {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if instance_pids()?.is_empty() {
            return Ok(true);
        }
        sleep(Duration::from_millis(100));
    }
    Ok(false)
}

fn stop_instances() -> Result<(), Stop> {
    let pids = instance_pids()?;
    if pids.is_empty() {
        return Ok(());
    }
    // Without `--pid`, `qs kill` only kills one instance. A failed kill is fine: the force kill
    // below catches any instance that doesn't go away.
    for pid in pids {
        let _ = qs().args(["kill", "--pid", &pid.to_string()]).stdout(Stdio::null()).status();
    }
    if wait_for_exit(Duration::from_secs(5))? {
        return Ok(());
    }
    warn("shell did not exit gracefully, killing");
    for pid in instance_pids()? {
        let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).stderr(Stdio::null()).status();
    }
    if wait_for_exit(Duration::from_secs(2))? { Ok(()) } else { Err(fail("an instance of the shell is still running")) }
}
