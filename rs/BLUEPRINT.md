# caelestia CLI (Rust) blueprint

Experimental Rust port of the caelestia CLI's hot commands. It runs beside the Python CLI and
`exec`s it for everything it doesn't port. Follows the applicable stages of Google's
`migrate-to-rust` skill; see "Skill stages" at the end.

## 1. Audit and baseline

Ported Python (about 1,300 lines): `parser.py`, `subcommands/{scheme,wallpaper}.py`,
`utils/{scheme,wallpaper,theme,paths,colour,hypr,notify,material/__init__}.py`.

Baseline (sandbox, median of 15, `rs/bench.sh`), before any Rust existed:

| command | Python ms |
| --- | --- |
| spawn floor (`/usr/bin/true`) | 0.28 |
| `scheme get` | 35.6 |
| `scheme list` | 37.6 |
| `scheme set --notify -m dark` | 51.7 |
| `wallpaper -p <jpg>` | 38.5 |
| `wallpaper -f <jpg>` | 53.9 |
| `wallpaper -f <mp4>` | 59.5 |
| `wallpaper -r <dir> -n` | 53.8 |

External floor: `sass` (Discord theme) takes about 9 ms and `dconf` about 2.5 ms. Both run on every
theme apply, so `scheme set` and `wallpaper -f` can't go below about 10 ms in any language.

Latent bugs found in the Python (ported as-is, not fixed):

1. `wallpaper -r -t X` without `-n` crashes. `--threshold` has no `type=float`, so
   `filter_size[0] * threshold` is `int * str` (string repetition) and the `>=` comparison raises
   `TypeError` (`utils/wallpaper.py` `check_wall`). The shell never passes `-t`.
2. `scheme set --notify -n <bad>` never notifies. argparse `choices` rejects the name (exit 2)
   before the `name` setter's notify branch can run.
3. `set_random`: the `ValueError("Only valid wallpaper is current")` is caught by its own
   `except ValueError`, so it ends in `random.choice([])` → `IndexError` instead.
4. `apply_colours`: when the lock is busy, the `finally` still unlinks `theme.lock`, deleting the
   holder's lock file, so a third concurrent run can take a fresh lock.
5. `wallpaper -r` compares unresolved `rglob` paths with the resolved `path.txt`, so the current
   wallpaper is not skipped when it is reached through a symlink or a relative directory.
6. `scheme list` gives both dynamic flavours the same colours: `get_colours_for_image()` uses the
   global scheme, not the per-flavour one.

## 2. Architecture and mapping

One binary crate, `rs/` (edition 2024). Deps: `serde_json` (`preserve_order`, parsing only),
`sha2` (SHA-NI file hashing for cache keys), `imagesize` (image headers for the `-r` size filter).
No clap (hand-rolled argv, about 100 lines). `unsafe`: only `setsid()` for the detached
papirus-folders call. The flock uses std's `File::try_lock`.

| Python | Rust |
| --- | --- |
| `parser.py`, `__init__.main` | `main.rs`: argv subset parser, dispatch, fallback `exec`, stdout buffering |
| `utils/paths.py` | `util.rs::Paths`, `atomic_write`, `hash_file` |
| `utils/scheme.py::Scheme` + `subcommands/scheme.py` | `scheme.rs` |
| `utils/material/__init__.py` (cache read only) | `scheme.rs::Ctx::image_colours` |
| `utils/wallpaper.py` + `subcommands/wallpaper.py` | `wallpaper.rs` |
| `utils/theme.py`, `utils/colour.py` | `theme.rs` |
| `utils/hypr.py` (`message`, `is_lua_config`), `utils/notify.py` | `util.rs` |
| `json.dumps` | `util.rs::dumps` (Python separators, `ensure_ascii`) |

## 3. Call-site design: plan, then act

Python mutates state as it goes (each `Scheme` setter saves `scheme.json`), so a Python fallback
halfway through would redo half-done work. Rust runs every command in two phases:

```
main.rs
  -> parse argv            (unknown or unported shape -> exec Python)
  -> plan: scheme/wallpaper logic with no side effects;
           records effects (writes, symlinks, saves, notifications) in order
           any missing cache, thumbnail, smart.json, first frame, odd JSON -> Stop::Fallback
  -> Fallback: drop the plan, exec Python with the same argv
  -> else: run the effects, then apply_colours / post hooks, flush stdout
```

`Stop::Fail` models Python's uncaught exceptions (exit 1) and runs the effects recorded before it,
as Python would have.

## 4. Parity strategy

`rs/parity.sh` (A/B, no unit or property suite; the harness is the test). For each case:
fresh sandbox copy at the same path → run Python → snapshot → fresh copy → run Rust → snapshot.
Compares stdout, exit code, every file under the sandbox (type, mode, symlink target, sha256),
the fake `/dev/pts` contents, and the sorted shim command log (sass temp dir names normalised).
A wrapper on `CAELESTIA_PY` records whether Rust fell back. Random cases (`scheme set -r`,
`wallpaper -r` with several candidates) are compared on exit code and shape only.

## 5. Benchmarks and PGO

Python vs Rust medians through `rs/bench.sh`. Then release with fat LTO plus `codegen-units=1`
plus `panic=abort`, and an `llvm-profdata` PGO build, kept only if it measurably beats plain release.

## 6. Roadmap

1. Sandbox, shims, baseline (done above).
2. `util.rs`, `scheme.rs` (get/list/set), `theme.rs`, `wallpaper.rs`, `main.rs`.
3. `parity.sh`. Iterate on diffs, logging each iteration below.
4. `bench.sh`, then PGO.

## Parity iteration log

| # | Result | Cause | Fix |
| --- | --- | --- | --- |
| 1 | 77/78 | `list-static-current` fell back where I expected native. With a static current scheme (`gruvbox medium`), `scheme list` asks for dynamic colours under flavour `medium`, which is never cached, and Python *generates and writes* that cache. Fallback is the correct behaviour. | Expectation fixed. Added `list-static-flavour-miss` (fallback) and made `list-static-current` use `caelestia default`, which is cached. |
| 2 | mutation check | Built a deliberately broken binary (Lua theme spacing, dconf `gtk-theme`). The harness flagged `commands files` on `set-latte`, so it does catch diffs. It also showed two harness bugs: the native/fallback counter (`((x++))` returns false at 0) and the content-diff column. | Both fixed in `parity.sh`. |
| 3 | 79/79 | 66 native, 13 fallback, every expectation met. | none |

Extra check: `imagesize` (used for the `-r` size filter) and Pillow give identical sizes for all
67 images in `~/Pictures/Wallpapers`.

## Results

Release build (fat LTO, `codegen-units=1`, `panic=abort`), `rs/bench.sh 21`, sandbox, median ms:

| command | Python | Rust | speedup |
| --- | --- | --- | --- |
| spawn floor | | 0.33 | |
| `scheme get` | 37.2 | 0.63 | 59x |
| `scheme list` | 39.5 | 2.0 | 20x |
| `scheme set --notify -m dark` | 52.9 | 10.1 | 5.3x |
| `wallpaper -p <jpg>` | 40.2 | 2.2 | 18x |
| `wallpaper -f <jpg>` | 55.8 | 12.2 | 4.6x |
| `wallpaper -f <mp4>` | 62.1 | 18.9 | 3.3x |
| `wallpaper -r <dir> -n` | 56.1 | 11.7 | 4.8x |

`set`/`-f`/`-r` sit on the `sass` floor (about 9 ms); `-p` and `-f` also hash the wallpaper
(SHA-NI; about 7 ms for the 11 MB video).

PGO (`-Cprofile-generate`, 30 representative runs, `llvm-profdata merge`, `-Cprofile-use`): no
measurable gain. Two interleaved rounds of 31 runs each landed within ±0.2 ms of the plain build
on every command, about the run-to-run noise. Startup and external processes dominate, so the
plain release build is kept and PGO isn't wired in.

## Skill stages

| Stage | Done? |
| --- | --- |
| 0 Baseline before porting | Yes: Python medians in the sandbox (section 1). No repo clone or confirmation gate (local code). |
| 1 Audit + blueprint, latent bugs | Yes: this file, 6 sections, 6 latent bugs. |
| 2.1 `#![forbid(unsafe_code)]` | Replaced per the brief: `unsafe` only for `setsid()`. flock uses safe `File::try_lock`. |
| 2.2 SoA/SIMD kernels, 400 LOC/file | SoA/SIMD skipped (I/O-bound CLI). Files are 200 to 660 lines; `theme.rs` is over 400 because it mirrors one 490-line Python module. |
| 2.3 Unit/doc/proptest, 95% coverage | Skipped per the brief: the A/B parity harness is the test. |
| 2.4 A/B parity with logged iterations | Yes: `parity.sh`, 79 cases, log above. |
| 2.5 PGO + final benchmarks | Yes: measured, no gain, not kept. |
| Telemetry JSON, HTML infographic | Skipped (optional; not cheap to wire in). |

## Running it

```
cd rs && cargo build --release
./parity.sh [case-regex]     # REBUILD=1 rebuilds the sandbox template under /tmp/caelestia-cli-sandbox
./bench.sh [runs] [binary]
```
