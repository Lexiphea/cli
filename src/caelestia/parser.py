import argparse


def parse_args() -> tuple[argparse.ArgumentParser, argparse.Namespace]:
    parser = argparse.ArgumentParser(prog="caelestia", description="Main control script for the Caelestia dotfiles")
    parser.add_argument("-v", "--version", action="store_true", help="print the current version")

    # Add subcommand parsers
    command_parser = parser.add_subparsers(
        title="subcommands", description="valid subcommands", metavar="COMMAND", help="the subcommand to run"
    )

    # Create parser for shell opts
    shell_parser = command_parser.add_parser("shell", help="start or message the shell")
    shell_parser.set_defaults(cmd="shell.Command")
    shell_parser.add_argument("message", nargs="*", help="a message to send to the shell")
    shell_parser.add_argument("-d", "--daemon", action="store_true", help="start the shell detached")
    shell_parser.add_argument("-r", "--restart", action="store_true", help="kill and restart the shell")
    shell_parser.add_argument("-s", "--show", action="store_true", help="print all shell IPC commands")
    shell_parser.add_argument("-l", "--log", action="store_true", help="print the shell log")
    shell_parser.add_argument("-k", "--kill", action="store_true", help="kill the shell")
    shell_parser.add_argument("--log-rules", metavar="RULES", help="log rules to apply")

    # `scheme` and `wallpaper` are handled by the Rust `caelestia` binary; listed here only for -h
    command_parser.add_parser("scheme", help="manage the colour scheme")
    command_parser.add_parser("wallpaper", help="manage the wallpaper")

    # Create parser for record opts
    record_parser = command_parser.add_parser("record", help="start a screen recording")
    record_parser.set_defaults(cmd="record.Command")
    record_parser.add_argument("-r", "--region", nargs="?", const="slurp", help="record a region")
    record_parser.add_argument("-s", "--sound", action="store_true", help="record audio")
    record_parser.add_argument("-p", "--pause", action="store_true", help="pause/resume the recording")
    record_parser.add_argument("-c", "--clipboard", action="store_true", help="copy recording path to clipboard")

    return parser, parser.parse_args()
