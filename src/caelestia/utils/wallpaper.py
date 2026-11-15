"""Wallpaper caches for `caelestia-py _gen wallpaper`: first frame, thumbnail and smart options."""

import json
import subprocess
from pathlib import Path
from typing import cast

from caelestia.utils.paths import compute_hash, wallpapers_cache_dir

VIDEO_EXTENSIONS = [".mp4", ".mkv", ".webm", ".mov", ".avi"]


def is_video(path: Path) -> bool:
    return path.suffix.lower() in VIDEO_EXTENSIONS


def is_animated(path: Path) -> bool:
    return path.suffix.lower() == ".gif" or is_video(path)


def get_thumb(wall: Path, cache: Path) -> Path:
    thumb = cache / "thumbnail.jpg"

    if not thumb.exists():
        from PIL import Image

        with Image.open(wall) as img:
            img = img.convert("RGB")
            img.thumbnail((128, 128), Image.Resampling.NEAREST)
            thumb.parent.mkdir(parents=True, exist_ok=True)
            img.save(thumb, "JPEG")

    return thumb


def get_smart_opts(wall: Path, cache: Path) -> dict:
    opts_cache = cache / "smart.json"

    try:
        return json.loads(opts_cache.read_text())
    except (IOError, json.JSONDecodeError):
        pass

    from materialyoucolor.hct import Hct
    from materialyoucolor.utils.color_utils import argb_from_rgb
    from PIL import Image

    from caelestia.utils.colourfulness import get_variant

    opts = {}

    with Image.open(get_thumb(wall, cache)) as img:
        opts["variant"] = get_variant(img)
        img.thumbnail((1, 1), Image.Resampling.LANCZOS)

        # Cast the pixel to a tuple of 3 integers to safely unpack it
        pixel = cast(tuple[int, int, int], img.getpixel((0, 0)))
        hct = Hct.from_int(argb_from_rgb(*pixel))

        opts["mode"] = "light" if hct.tone > 60 else "dark"

    opts_cache.parent.mkdir(parents=True, exist_ok=True)
    with opts_cache.open("w") as f:
        json.dump(opts, f)

    return opts


def extract_first_frame(wall: Path, cache: Path | None = None) -> Path:
    cache = cache or wallpapers_cache_dir / compute_hash(wall)
    output_path = cache / "first_frame.png"

    if output_path.exists():
        return output_path

    output_path.parent.mkdir(parents=True, exist_ok=True)

    if is_video(wall):
        subprocess.run(
            ["ffmpeg", "-y", "-loglevel", "error", "-i", wall, "-frames:v", "1", output_path],
            check=True,
            stdin=subprocess.DEVNULL,
        )
        return output_path

    from PIL import Image

    with Image.open(wall) as img:
        try:
            img.seek(0)
        except EOFError:
            pass

        img = img.convert("RGB")
        img.save(output_path, "PNG")

    return output_path


def cache_wallpaper(wall: Path) -> None:
    """The cache files the Rust CLI reads: under the first frame's hash for animated wallpapers."""
    cache = wallpapers_cache_dir / compute_hash(wall)
    if is_animated(wall):
        wall = extract_first_frame(wall, cache)
        cache = wallpapers_cache_dir / compute_hash(wall)
    get_thumb(wall, cache)
    get_smart_opts(wall, cache)
