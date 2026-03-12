# zellij-tools

A [Zellij](https://github.com/zellij-org/zellij) plugin and companion CLI that add scratchpads, focus helpers, event streaming, and session tree utilities.

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
            popup {
                command "zsh"
                width "80%"
                height "60%"
                origin "center"
                title "Popup Shell"
                cwd "/home/user/projects"
            }
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
    btop {
        command "btop"
        width "120"
        height "40"
        origin "center"
    }
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

### Scratchpad Options

Each scratchpad supports these options:

| Option    | Description                                                                     |    Required     |
| --------- | ------------------------------------------------------------------------------- | :-------------: |
| `command` | Command and arguments to run (e.g. `command "zsh"` or `command "nvim" "+cd ~"`) |       Yes       |
| `width`   | Pane width: fixed columns (`"80"`) or percent (`"50%"`)                         |       No        |
| `height`  | Pane height: fixed rows (`"24"`) or percent (`"50%"`)                           |       No        |
| `x`       | Horizontal offset: fixed columns or percent                                     |       No        |
| `y`       | Vertical offset: fixed rows or percent                                          |       No        |
| `origin`  | Anchor point for x/y coordinates (see below)                                    |   `"center"`    |
| `title`   | Pane title displayed in the Zellij UI                                           | Scratchpad name |
| `cwd`     | Working directory for the command                                               |       No        |

### Origin

The `origin` option sets the reference point for `x` and `y` coordinates. It accepts one or two arguments:

- **One argument:** `"center"` (both axes), `"top"`, `"bottom"`, `"left"`, `"right"`
- **Two arguments:** vertical then horizontal, e.g. `origin "bottom" "center"`, `origin "top" "right"`

| Vertical | Horizontal | Description                              |
| -------- | ---------- | ---------------------------------------- |
| `top`    | `left`     | Offset from top-left corner              |
| `center` | `center`   | Centered, offset shifts away from center |
| `bottom` | `right`    | Offset inward from bottom-right corner   |

### Scratchpad CLI

Control scratchpads from the command line:

```sh
zellij-tools scratchpad toggle         # Toggle the last-focused scratchpad
zellij-tools scratchpad toggle term    # Toggle a named scratchpad
zellij-tools scratchpad show term      # Show a scratchpad
zellij-tools scratchpad hide term      # Hide a scratchpad
zellij-tools scratchpad close term     # Close a scratchpad (terminates the pane)
```

## Other Actions

### Focus Pane

Focuses a pane by ID. You can get the pane ID from the `$ZELLIJ_PANE_ID` environment variable.

Note: pane IDs are only unique within their type. A `terminal` id `0` and a `plugin` id `0` can both exist at the same time.

Defaults to terminal panes in the CLI. Use `--plugin` to target plugin panes.

```sh
zellij-tools focus pane 2
zellij-tools focus pane --plugin 7
```

### Focus Tab

Focuses a tab by position (1-based) by default.

Use `--id` to focus by stable tab ID.

```sh
zellij-tools focus tab 2
zellij-tools focus tab --id 42
```

## Events and Tree

Stream pane/tab events:

```sh
zellij-tools subscribe
zellij-tools subscribe --full
zellij-tools subscribe --event PaneFocused,TabMoved --pane-id 2 --plugin-pane-id 7 --tab-id 42
```

Get a session tree snapshot:

```sh
zellij-tools tree
```

For full event formats and examples, see `docs/event-subscription-api.md`.

## Permissions

The plugin requires the following permissions:

- `ReadApplicationState` - Track panes and tabs
- `ChangeApplicationState` - Show/hide panes
- `RunCommands` - Launch scratchpad commands
- `ReadCliPipes` - Stream events and tree data to CLI pipes
- `FullHdAccess` - Read external config files

## License

&copy; 2025 Maddison Hellstrom

MIT License
