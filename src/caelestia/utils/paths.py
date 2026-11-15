import hashlib
import json
import os
from pathlib import Path
from typing import Any

from caelestia.utils.io import warn

config_dir: Path = Path(os.getenv("XDG_CONFIG_HOME", Path.home() / ".config"))
state_dir: Path = Path(os.getenv("XDG_STATE_HOME", Path.home() / ".local/state"))
cache_dir: Path = Path(os.getenv("XDG_CACHE_HOME", Path.home() / ".cache"))
videos_dir: Path = Path(os.getenv("XDG_VIDEOS_DIR", Path.home() / "Videos"))

c_state_dir: Path = state_dir / "caelestia"
c_cache_dir: Path = cache_dir / "caelestia"

user_config_path: Path = config_dir / "caelestia/cli.json"

scheme_cache_dir: Path = c_cache_dir / "schemes"
wallpapers_cache_dir: Path = c_cache_dir / "wallpapers"

recordings_dir: Path = Path(os.getenv("CAELESTIA_RECORDINGS_DIR", videos_dir / "Recordings"))
recording_path: Path = c_state_dir / "record/recording.mp4"
recording_notif_path: Path = c_state_dir / "record/notifid.txt"


def compute_hash(path: Path | str) -> str:
    with open(path, "rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def get_config() -> dict[str, Any]:
    try:
        return json.loads(user_config_path.read_text())
    except json.JSONDecodeError:
        warn("failed to parse config, invalid JSON")
    except FileNotFoundError:
        pass
    return {}
