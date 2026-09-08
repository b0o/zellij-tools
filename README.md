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

### Nix Flake

The plugin and CLI are available as flake outputs:

```sh
# Build the plugin
nix build github:b0o/zellij-tools          # → result/share/zellij/plugins/zellij-tools.wasm

# Build the CLI
nix build github:b0o/zellij-tools#cli      # → result/bin/zellij-tools

# Run the CLI without installing
nix run github:b0o/zellij-tools#cli -- scratchpad list

# Enter a dev shell with the Rust toolchain and CLI
nix develop github:b0o/zellij-tools
```

For local development, the flake also exposes Cargo-backed apps that keep using
the workspace `target/` directory for incremental builds:

```sh
nix run .#build       # Build the plugin with cargo
nix run .#build-cli   # Build the CLI with cargo
nix run .#check       # Run dprint and clippy checks
nix run .#test        # Run library tests
nix run .#fmt         # Format sources
nix run . -- --help   # Run the CLI through cargo
```

To use the plugin from your Nix-managed Zellij config, add the flake as an input and reference the wasm path:

```nix
# flake.nix
{
  inputs.zellij-tools.url = "github:b0o/zellij-tools";

  # ...in your outputs:
  # The plugin wasm is at:
  #   zellij-tools.packages.${system}.default + "/share/zellij/plugins/zellij-tools.wasm"
  # The CLI binary is at:
  #   zellij-tools.packages.${system}.cli + "/bin/zellij-tools"
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
    term {
        command "zsh"
        keybinds {
            shared_among "normal" "locked" {
                bind "Ctrl Shift D" { Toggle; SwitchToMode "locked"; }
            }
        }
    }
    btop {
        command "btop"
        width "120"
        height "40"
        origin "center"
    }
}

// Optional: publish scratchpad status to the zjstatus plugin.
zjstatus {
    pipe "scratchpads"
    format "{current_items}"
    current_item_visible_format "#[fg=green]{title}"
    current_item_hidden_format "#[fg=gray]{title}"
    current_item_closed_format ""
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

| Option        | Description                                                | Default       | Inline Config | External Config File |
| ------------- | ---------------------------------------------------------- | ------------- | :-----------: | :------------------: |
| `include`     | Path to external config file                               | -             |      Yes      |          No          |
| `config_dir`  | Override base directory for relative includes              | Auto-detected |      Yes      |          No          |
| `watch_ms`    | Polling interval in ms. `"false"` or `"0"` to disable.     | `2000`        |      Yes      |          No          |
| `scratchpads` | Scratchpad definitions                                     | -             |      Yes      |         Yes          |
| `zjstatus`    | Optional scratchpad status output for the zjstatus plugin  | -             |      Yes      |         Yes          |

### Scratchpad Options

Each scratchpad supports these options:

| Option     | Description                                                                                              |    Required     |
| ---------- | -------------------------------------------------------------------------------------------------------- | :-------------: |
| `command`  | Command and arguments to run (e.g. `command "zsh"` or `command "nvim" "+cd ~"`)                          |       Yes       |
| `width`    | Pane width: fixed columns (`"80"`) or percent (`"50%"`)                                                  |       No        |
| `height`   | Pane height: fixed rows (`"24"`) or percent (`"50%"`)                                                    |       No        |
| `x`        | Horizontal offset: fixed columns or percent                                                              |       No        |
| `y`        | Vertical offset: fixed rows or percent                                                                   |       No        |
| `origin`   | Anchor point for x/y coordinates (see below)                                                             |   `"center"`    |
| `title`    | Pane title displayed in the Zellij UI                                                                    | Scratchpad name |
| `cwd`      | Working directory for the command                                                                        |       No        |
| `keybinds` | Client-local keybindings to trigger scratchpad actions (see [Scratchpad Keybinds](#scratchpad-keybinds)) |       No        |

### Origin

The `origin` option sets the reference point for `x` and `y` coordinates. It accepts one or two arguments:

- **One argument:** `"center"` (both axes), `"top"`, `"bottom"`, `"left"`, `"right"`
- **Two arguments:** vertical then horizontal, e.g. `origin "bottom" "center"`, `origin "top" "right"`

| Vertical | Horizontal | Description                              |
| -------- | ---------- | ---------------------------------------- |
| `top`    | `left`     | Offset from top-left corner              |
| `center` | `center`   | Centered, offset shifts away from center |
| `bottom` | `right`    | Offset inward from bottom-right corner   |

### Scratchpad Keybinds

Scratchpad keybinds are configured inside each scratchpad definition using a `keybinds` block:

```kdl
term {
    command "nu"
    keybinds {
        shared_among "normal" "locked" {
            bind "Ctrl Shift D" { Toggle; SwitchToMode "locked"; }
        }
    }
}
```

Unlike the old approach of putting `MessagePlugin { ... }` bindings in Zellij's global keybinds block, scratchpad-local keybinds are installed by the plugin at runtime and route to the invoking client only. This means they work correctly in multi-client sessions.

#### Supported actions

| Action         | Description                                                                                              |
| -------------- | -------------------------------------------------------------------------------------------------------- |
| `Toggle`       | Toggle the scratchpad (show if hidden, hide if visible)                                                  |
| `Show`         | Show the scratchpad                                                                                      |
| `Hide`         | Hide the scratchpad                                                                                      |
| `Close`        | Close the scratchpad (terminates the pane)                                                               |
| `SwitchToMode` | Optional convenience action to switch Zellij's input mode after the action, e.g. `SwitchToMode "locked"` |

#### Mode blocks

Keybinds can be active in one or more Zellij input modes:

```kdl
keybinds {
    normal { bind "Alt t" { Toggle; } }                          // active in normal mode only
    shared_among "locked" "normal" { bind "Alt s" { Show; } }    // active in locked and normal modes
    shared_except "scroll" { bind "Alt h" { Hide; } }            // active in all modes except scroll
    shared { bind "Ctrl Space" { Toggle; } }                     // active in all modes
}
```

Supported modes: `normal`, `locked`, `resize`, `pane`, `move`, `tab`, `scroll`, `search`, `entersearch`, `renametab`, `renamepane`, `session`, `tmux`.

#### Migration from old MessagePlugin bindings

Previously, scratchpad keybinds required adding `MessagePlugin` blocks to Zellij's global keybinds config:

```kdl
// OLD — remove these from config.kdl
keybinds {
    shared_except "resize" "scroll" {
        bind "Ctrl Shift D" {
            MessagePlugin "zellij-tools" { payload "zellij-tools::scratchpad::toggle::term"; }
            SwitchToMode "locked"
        }
    }
}
```

Replace those with `keybinds { ... }` blocks inside each scratchpad definition in your `zellij-tools.kdl` file, as shown above. Then remove the old `MessagePlugin` bindings from `config.kdl`.

### zjstatus Integration

The plugin can publish scratchpad status to [zjstatus](https://github.com/dj95/zjstatus) using zjstatus' pipe protocol. Add a `zjstatus` block to the inline plugin config or to the external file loaded by `include`:

```kdl
// inside of the zellij-tools config:
zjstatus {
    // Publishes to zjstatus' pipe_scratchpads widget.
    pipe "scratchpads"

    // Top-level output shown by zjstatus.
    format "{current_items} #[fg=#666666]+{other_live_count}"

    // Optional fallback published when the rendered output is empty.
    // Leave empty to avoid clearing the previous zjstatus output during transient updates.
    empty_format ""

    current_item_focused_format "#[fg=#7583FF,bold]{title}"
    current_item_visible_format "#[fg=#585d8d,bold]{title}"
    current_item_hidden_format "#[fg=#666666]{title}"
    current_item_closed_format ""
    item_separator " "
}
```

Then configure zjstatus to render the matching pipe widget:

```kdl
zjstatus location="https://github.com/dj95/zjstatus/releases/latest/download/zjstatus.wasm" {
    pipe_scratchpads_format "{output} "
    pipe_scratchpads_rendermode "dynamic"
    format_right "{pipe_scratchpads} {datetime} {session}"
}
```

The plugin publishes updates when scratchpads change, when panes/tabs change, when a zjstatus plugin pane appears, and when it receives `zellij-tools::zjstatus::refresh`.

#### zjstatus Placeholders

Top-level `format` supports these placeholders:

| Placeholder                                                                           | Description                                                   |
| ------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| `{current_items}`, `{global_items}`                                                   | Rendered scratchpad items for the active tab or whole session |
| `{current_configured_count}`, `{global_configured_count}`, `{other_configured_count}` | Configured scratchpad counts after include/exclude filters    |
| `{current_rendered_count}`, `{global_rendered_count}`, `{other_rendered_count}`       | Items that render after empty item formats are filtered out   |
| `{current_live_count}`, `{global_live_count}`, `{other_live_count}`                   | Scratchpads with live panes                                   |
| `{current_visible_count}`, `{global_visible_count}`, `{other_visible_count}`          | Visible scratchpad counts                                     |
| `{current_hidden_count}`, `{global_hidden_count}`, `{other_hidden_count}`             | Hidden/suppressed scratchpad counts                           |
| `{current_closed_count}`, `{global_closed_count}`, `{other_closed_count}`             | Configured scratchpads without a live pane                    |
| `{current_focused_name}`, `{global_focused_name}`                                     | Focused scratchpad name                                       |
| `{current_focused_title}`, `{global_focused_title}`                                   | Focused scratchpad title                                      |

`current_*` values describe the active tab. `global_*` values describe the whole session. `other_*` values are `global` minus `current`.

Item formats support these placeholders:

| Placeholder             | Description                                |
| ----------------------- | ------------------------------------------ |
| `{name}`                | Scratchpad config name                     |
| `{title}`               | Configured title, falling back to the name |
| `{state}`               | `visible`, `hidden`, or `closed`           |
| `{icon}`                | Fixed state icon: `●`, `○`, or `×`         |
| `{pane_id}`, `{tab_id}` | Live pane and native Zellij tab IDs        |
| `{tab_position}`        | 1-based tab position when available        |
| `{is_focused}`          | `true` or `false`                          |

Item format fallback order is: `<scope>_item_<state>_format`, `item_<state>_format`, `<scope>_item_format`, then `item_format`. The `current_item_focused_format` override wins for focused scratchpads on the active tab. Set a state-specific item format to an empty string to omit those items.

Supported zjstatus config keys are: `pipe`, `format`, `empty_format`, `item_format`, `item_visible_format`, `item_hidden_format`, `item_closed_format`, `current_item_format`, `current_item_focused_format`, `current_item_visible_format`, `current_item_hidden_format`, `current_item_closed_format`, `global_item_format`, `global_item_visible_format`, `global_item_hidden_format`, `global_item_closed_format`, `item_separator`, `current_item_separator`, `global_item_separator`, `include`, and `exclude`.

### Scratchpad CLI

Control scratchpads from the command line:

```sh
zellij-tools scratchpad toggle       # Toggle the last-focused scratchpad
zellij-tools scratchpad toggle term  # Toggle a named scratchpad
zellij-tools scratchpad toggle term --tab 7  # Toggle a scratchpad on a specific tab
zellij-tools scratchpad toggle term --current-tab  # Toggle a scratchpad on the focused tab
zellij-tools scratchpad show term    # Show a scratchpad
zellij-tools scratchpad hide term    # Hide a scratchpad
zellij-tools scratchpad close term   # Close a scratchpad (terminates the pane)
zellij-tools scratchpad list         # List configured scratchpads as JSON
zellij-tools scratchpad list --full  # Include full pane info for live instances
```

If `ZELLIJ_PANE_ID` is set in your environment (automatic inside Zellij) and no `--tab` or `--current-tab` is provided, the CLI infers the target tab from the calling pane. Otherwise, the receiving plugin instance's current tab is used (may be ambiguous in multi-client sessions). Use `--current-tab` to explicitly target the focused tab. `--tab-id` is accepted as an alias for `--tab`.

`scratchpad list` accepts optional scratchpad names and the same `--tab`, `--tab-id`, and `--current-tab` filters as the control commands. It returns JSON sorted by scratchpad name and includes orphaned scratchpads that still have panes after being removed from config.

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

Use `--id` to focus by tab ID.

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
zellij-tools tree --tab 42
zellij-tools tree --current-tab
```

For full event formats and filter options, see `zellij-tools subscribe --help`.

## Permissions

The plugin requires the following permissions:

- `ReadApplicationState` - Track panes and tabs
- `ChangeApplicationState` - Show/hide panes
- `RunCommands` - Launch scratchpad commands
- `ReadCliPipes` - Stream events and tree data to CLI pipes
- `FullHdAccess` - Read external config files
- `Reconfigure` - Install scratchpad keybinds at runtime
- `MessageAndLaunchOtherPlugins` - Publish scratchpad status to zjstatus

## License

&copy; 2025-2026 Maddison Cohodas

MIT License
