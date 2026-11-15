import sys


def main() -> None:
    # The Rust `caelestia` binary is the CLI; this only fills its colour caches (`_gen`)
    if sys.argv[1:2] != ["_gen"]:
        sys.exit("caelestia-py only generates caches for the caelestia binary: caelestia-py _gen ...")

    from caelestia.generate import main as generate

    generate(sys.argv[2:])
