import hashlib
import os
from pathlib import Path

cache_dir: Path = Path(os.getenv("XDG_CACHE_HOME", Path.home() / ".cache"))
c_cache_dir: Path = cache_dir / "caelestia"

scheme_cache_dir: Path = c_cache_dir / "schemes"
wallpapers_cache_dir: Path = c_cache_dir / "wallpapers"


def compute_hash(path: Path | str) -> str:
    with open(path, "rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()
