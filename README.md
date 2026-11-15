# caelestia-cli

The main control script for the Caelestia dotfiles.

<details><summary id="dependencies">External dependencies</summary>

- [`libnotify`](https://gitlab.gnome.org/GNOME/libnotify) - sending notifications
- [`dart-sass`](https://github.com/sass/dart-sass) - discord theming
- [`wl-clipboard`](https://github.com/bugaevc/wl-clipboard) - copying to clipboard
- [`slurp`](https://github.com/emersion/slurp) - selecting a recording area
- [`gpu-screen-recorder`](https://git.dec05eba.com/gpu-screen-recorder/about) - screen recording
- `glib2` - closing notifications
- [`ffmpeg`](https://ffmpeg.org) - video wallpaper detection and colour generation

</details>

## Installation

This is a Rust port of upstream's Python CLI, vendored in the dotfiles. Install [`rust`](https://www.rust-lang.org)
and the [dependencies](#dependencies), then build it and link the binary onto your `PATH`:

```sh
cargo build --release --manifest-path rs/Cargo.toml
ln -s "$PWD/rs/target/release/caelestia" ~/.local/bin/caelestia
sudo cp completions/caelestia.fish /usr/share/fish/vendor_completions.d/caelestia.fish
```

The binary reads its templates from `src/caelestia/data` in this checkout, so keep it where it was built.
`setup-dotfiles.sh` does all of this, and its git hook rebuilds the binary after pulls that change `rs/`.

### Additional steps

#### Auto folder colour theming

For automatic Papirus folder icon colour syncing, you must have [`papirus-folders`](https://github.com/PapirusDevelopmentTeam/papirus-folders)
installed, and `papirus-folders` must to be able to run with `sudo` without a password prompt.

You can allow this by creating a sudoers file:

```sh
echo "$USER ALL=(ALL) NOPASSWD: $(which papirus-folders)" | sudo tee /etc/sudoers.d/papirus-folders
sudo chmod 440 /etc/sudoers.d/papirus-folders
```

#### Chromium-based browser theming

For live Chromium-based browser theming, the CLI must be allowed to create certain directories in `/etc`
and write to them via `sudo` without a password prompt.

You can allow this by creating a sudoers file:

```fish
# Fish shell
for dir in /etc/chromium/policies/managed /etc/brave/policies/managed /etc/opt/chrome/policies/managed
    echo "$USER ALL=(ALL) NOPASSWD: $(which mkdir) -p $dir" | sudo tee -a /etc/sudoers.d/caelestia-chromium
    echo "$USER ALL=(ALL) NOPASSWD: $(which tee) $dir/caelestia.json" | sudo tee -a /etc/sudoers.d/caelestia-chromium
end
sudo chmod 440 /etc/sudoers.d/caelestia-chromium
```

```sh
# Bash/other shells
for dir in /etc/chromium/policies/managed /etc/brave/policies/managed /etc/opt/chrome/policies/managed; do
    echo "$USER ALL=(ALL) NOPASSWD: $(which mkdir) -p $dir" | sudo tee -a /etc/sudoers.d/caelestia-chromium
    echo "$USER ALL=(ALL) NOPASSWD: $(which tee) $dir/caelestia.json" | sudo tee -a /etc/sudoers.d/caelestia-chromium
done
sudo chmod 440 /etc/sudoers.d/caelestia-chromium
```

## Usage

All subcommands/options can be explored via the help flag.

```
$ caelestia -h
usage: caelestia [-h] [-v] COMMAND ...

Main control script for the Caelestia dotfiles

options:
  -h, --help     show this help message and exit
  -v, --version  print the current version

subcommands:
  valid subcommands

  COMMAND        the subcommand to run
    shell        start or message the shell
    scheme       manage the colour scheme
    record       start a screen recording
    wallpaper    manage the wallpaper
```

### User templates

Custom user templates can be defined in `~/.config/caelestia/templates/`.

#### Template syntax

`{{ <color>.<format> }}`

- `<color>` is a theme color role derived from the Material You color system (e.g. `primary`, `secondary`, `background`)
- `<format>` is the output format: `hex`, `rgb`, `hsl`, or a single channel (`red`, `green`, `blue`, `hue`, `saturation`, `lightness`)

#### Examples

- `{{ primary.hex }}` outputs `3f4ba2`
- `{{ primary.rgb }}` outputs `rgb(193, 132, 207)`
- `{{ primary.red }}` outputs `193`
- `{{ primary.hsl }}` outputs `hsl(268,41%,66%)`
- `{{ primary.hue }}` outputs `268`

Output files are written to `~/.local/state/caelestia/theme/`. You can symlink them to your desired locations.

## Configuring

All configuration options are in `~/.config/caelestia/cli.json`.

<details><summary>Example configuration</summary>

```json
{
    "record": {
        "extraArgs": []
    },
    "wallpaper": {
        "postHook": "echo $WALLPAPER_PATH $SCHEME_NAME $SCHEME_FLAVOUR $SCHEME_MODE $SCHEME_VARIANT $SCHEME_COLOURS"
    },
    "theme": {
        "enableTerm": true,
        "enableHypr": true,
        "enableDiscord": true,
        "discordClients": ["Equicord", "Vencord", "BetterDiscord", "equibop", "vesktop", "legcord"],
        "enableSpicetify": true,
        "enablePandora": true,
        "enableFuzzel": true,
        "enableBtop": true,
        "enableNvtop": true,
        "enableHtop": true,
        "enableGtk": true,
        "enableQt": true,
        "enableWarp": true,
        "enableChromium": true,
        "enableZed": true,
        "enableCava": true,
        "iconTheme": "Papirus-Dark",
        "iconThemeLight": "Papirus-Light",
        "iconThemeDark": "Papirus-Dark",
        "postHook": "echo $SCHEME_NAME $SCHEME_FLAVOUR $SCHEME_MODE $SCHEME_VARIANT $SCHEME_COLOURS"
    }
}
```

</details>
