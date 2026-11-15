import sys
from importlib import import_module

from caelestia.parser import parse_args
from caelestia.utils.io import log


def main() -> None:
    if sys.argv[1:2] == ["_gen"]:
        from caelestia.generate import main as generate

        return generate(sys.argv[2:])
    try:
        parser, args = parse_args()
        if args.version:
            from caelestia.utils.version import print_version

            print_version()
        elif "cmd" in args:
            module, cls = args.cmd.rsplit(".", 1)
            getattr(import_module(f"caelestia.subcommands.{module}"), cls)(args).run()
        else:
            parser.print_help()
    except KeyboardInterrupt:
        log("Exiting...")
