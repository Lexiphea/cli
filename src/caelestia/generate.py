"""Fills the colour caches for the Rust `caelestia` binary, which owns schemes, wallpapers and themes.

Writes only cache files, never state or themes:
    caelestia-py _gen wallpaper PATH                      first frame, thumbnail and smart.json
    caelestia-py _gen colours IMAGE VARIANT FLAVOUR MODE  the dynamic scheme for a thumbnail
"""

import sys
from pathlib import Path
from types import SimpleNamespace


def main(args: list[str]) -> None:
    match args:
        case ["wallpaper", wall]:
            from caelestia.utils.wallpaper import cache_wallpaper

            cache_wallpaper(Path(wall))
        case ["colours", image, variant, flavour, mode]:
            from caelestia.utils.material import get_colours_for_image

            get_colours_for_image(image, SimpleNamespace(variant=variant, flavour=flavour, mode=mode))
        case _:
            sys.exit(__doc__)
