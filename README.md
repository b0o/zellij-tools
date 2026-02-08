# zellij-tools

A [Zellij](https://github.com/zellij-org/zellij) plugin that adds handy utilities including scratchpad terminals.

## Installation

```kdl
plugins {
    zellij-tools location="https://github.com/b0o/zellij-tools/releases/latest/download/zellij-tools.wasm"
}

load_plugins {
    zellij-tools
}
```

## Scratchpads

Scratchpads are floating terminal panes that can be quickly toggled on and off. They follow you across tabs and persist their state.

### Configuration

Scratchpads can be configured inline or in an external file.

**Important:** Inline configuration requires restarting zellij to apply changes. For hot-reloading, use an external config file.

#### Inline Configuration (no hot-reload)

```kdl
plugins {
    zellij-tools location="..." {
        scratchpads {
            term { command "zsh"; }
            btop { command "btop"; }
            notes { command "nvim" "+cd ~/notes"; }
        }
    }
}
```

#### External Configuration File (hot-reload supported)

Use an external file to edit scratchpad definitions without restarting zellij:

```kdl
plugins {
    zellij-tools location="..." {
        include "zellij-tools.kdl"  // Relative to zellij config directory
        // config_dir "~/.config/zellij"  // Override base directory for relative includes
        // watch_ms "2000"  // Polling interval in ms, or "false"/"0" to disable
    }
}
```

Then create `~/.config/zellij/zellij-tools.kdl`:

```kdl
scratchpads {
    term { command "zsh"; }
    btop { command "btop"; }
}
```

The plugin polls the external file for changes and automatically reloads scratchpad definitions.

### Include Path Resolution

The `include` path is resolved as follows:

- Absolute paths (starting with `/`) are used as-is
- Paths starting with `~` are expanded to your home directory
- Relative paths are resolved against your zellij config directory

The config directory is determined by (in order):

1. `ZELLIJ_CONFIG_DIR` environment variable
2. `$XDG_CONFIG_HOME/zellij`
3. `$HOME/.config/zellij`

### Configuration Options

| Option        | Description                                            | Default       | Inline Config | External Config File |
| ------------- | ------------------------------------------------------ | ------------- | :-----------: | :------------------: |
| `include`     | Path to external config file                           | -             |      Yes      |          No          |
| `config_dir`  | Override base directory for relative includes          | Auto-detected |      Yes      |          No          |
| `watch_ms`    | Polling interval in ms. `"false"` or `"0"` to disable. | `2000`        |      Yes      |          No          |
| `scratchpads` | Scratchpad definitions                                 | -             |      Yes      |         Yes          |

### Usage

Toggle a scratchpad:

```sh
zellij pipe "zellij-tools::scratchpad::toggle::term"
```

Or bind it to a key:

```kdl
bind "Alt t" {
    MessagePlugin "zellij-tools" {
        payload "zellij-tools::scratchpad::toggle::term"
    }
}
```

### Scratchpad Actions

| Action           | Description                      |
| ---------------- | -------------------------------- |
| `toggle::<name>` | Toggle scratchpad visibility     |
| `show::<name>`   | Show scratchpad                  |
| `hide::<name>`   | Hide scratchpad                  |
| `close::<name>`  | Close scratchpad (process exits) |

## Other Actions

### Focus Pane

Focuses a pane by ID. You can get the pane ID from the `$ZELLIJ_PANE_ID` environment variable.

Defaults to terminal panes in the CLI. Use `--plugin` to target plugin panes.

```sh
zellij pipe "zellij-tools::focus-pane::2"
zellij-tools focus pane 2
zellij-tools focus pane 7 --plugin
```

### Focus Tab

Focuses a tab by position (1-based) by default.

Use `--id` to focus by stable tab ID.

```sh
zellij pipe "zellij-tools::focus-tab::2"
zellij-tools focus tab 2
zellij-tools focus tab 42 --id
```

## Permissions

The plugin requires the following permissions:

- `ReadApplicationState` - Track panes and tabs
- `ChangeApplicationState` - Show/hide panes
- `RunCommands` - Launch scratchpad commands
- `FullHdAccess` - Read external config files

## License

&copy; 2025 Maddison Hellstrom

MIT License
