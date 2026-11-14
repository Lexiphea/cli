from importlib.metadata import version


def print_version() -> None:
    # The nexus About page parses `caelestia-cli <version>` from this line
    print(f"caelestia-cli {version('caelestia')}")
