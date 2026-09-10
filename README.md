# zellij-tools

A [Zellij](https://github.com/zellij-org/zellij) plugin and companion CLI that add scratchpads, pane status, focus helpers, event streaming, and session tree utilities.

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

Scratchpads are floating terminal panes that can be quickly toggled on and off. Instances are keyed by scratchpad name and native tab ID, so the same name can have independent panes on different tabs. Hiding preserves the pane; closing terminates it.

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
    pipe "scratchpads" {
        format "{current_items}"
        current_item_visible_format "#[fg=green]{title}"
        current_item_hidden_format "#[fg=gray]{title}"
        current_item_closed_format ""
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
| `zjstatus`    | Optional scratchpad or pane status output for zjstatus | -             |      Yes      |         Yes          |

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

The plugin publishes global widgets and independent per-tab fields using the zjstatus pipe protocol. Global widgets work with [upstream zjstatus](https://github.com/dj95/zjstatus); tab fields require a build with the tab-pipe extension, such as the local fork used in `dev.kdl`. Add a `zjstatus` block inline or in the external file loaded by `include`.

#### Breaking Migration

Only nested `pipe "name" { ... }` and `tab_pipe "field" { ... }` outputs are supported. The old flat `pipe "scratchpads"` followed by sibling formatting options is rejected, including when mixed with new syntax. Move those options inside the pipe block; the receiver's global widget name stays unchanged. Minimal replacement:

```kdl
zjstatus {
    pipe "scratchpads" {
        format "{current_items}"
        current_item_closed_format ""
    }
}
```

#### Multiple Outputs

```kdl
zjstatus {
    refresh_ms 2000

    pipe "scratchpads" {
        format "{current_items} #[fg=#666666]+{other_live_count}"
        current_item_focused_format "#[fg=#7583FF,bold]{title}"
        current_item_visible_format "#[fg=#585d8d,bold]{title}"
        current_item_hidden_format "#[fg=#666666]{title}"
        current_item_closed_format ""
    }

    tab_pipe "scratchpads" {
        format "{tab_items}"
        include "term" "git"
        item_visible_format "{name}+"
        item_hidden_format "{name}-"
        item_closed_format ""
    }

    tab_pipe "notes" {
        include "notes"
        item_visible_format "#[fg=green,bg=black,bold]{title}"
        item_hidden_format "#[fg=gray,bg=black]{title}"
        item_closed_format ""
    }
}
```

Configure the tab-pipe-capable receiver with matching names. Static mode styles a wrapper around plain producer text; dynamic mode interprets producer-authored zjstatus styles:

```kdl
zjstatus location="file:/absolute/path/to/tab-pipe-capable/zjstatus.wasm" {
    pipe_scratchpads_format "{output} "
    pipe_scratchpads_rendermode "dynamic"
    format_right "{pipe_scratchpads} {datetime} {session}"
    format_left "{tabs}"
    tab_normal "{index} {name}{tab_pipe_scratchpads}{tab_pipe_notes}"
    tab_active "#[bold]{index} {name}{tab_pipe_scratchpads}{tab_pipe_notes}"
    tab_pipe_scratchpads_format "#[fg=blue,bg=black] [{output}]"
    tab_pipe_scratchpads_rendermode "static"
    tab_pipe_notes_format " ({output})"
    tab_pipe_notes_rendermode "dynamic"
}
```

Tab placeholders belong in tab label formats, including any explicit bell/fullscreen/sync/rename variants, not outer bar formats. Tab pipe styles do not inherit the surrounding tab style. Missing or cleared tab values hide their wrappers.

#### Schema and Layering

- `source "scratchpad"` is the default on both output kinds. Use `source "pane-status"` for [pane status outputs](#pane-status-outputs); the scratchpad-specific defaults and options below do not apply to that source.
- Repeat either output kind as needed. Identity is `(kind, name)`, so global `scratchpads` and tab `scratchpads` coexist; duplicate identities within one layer are errors.
- Global names match `[A-Za-z0-9_-]+`; tab field names match `[a-z0-9_]+`. Use the bare name, without `pipe_` or `tab_pipe_`.
- `enabled true` is the default. `enabled false` disables an output; it is a KDL boolean, not a string.
- Each output has independent defaults: `format "{current_items}"` for global pipes, `format "{tab_items}"` for tab pipes, `item_format "{icon} {name}"`, `item_separator " "`, and `empty_format ""`. There is no cross-output inheritance.
- Inline and external definitions merge by identity. Explicit external fields win; omitted fields retain inline values. Defaults apply after merging. Omitting an external output does not delete its inline definition; use `enabled false` to disable it.
- `include` and `exclude` are exact scratchpad-name lists applied before counts. Exclude wins. Lists replace rather than append; `include;` resets to all names and `exclude;` resets to no exclusions. Items are alphabetical, not ordered by the include list.
- Explicit empty strings override inherited formats. Empty item formats omit those items without extra separators and affect `rendered_count`, not `live_count`.

For example, an external override can disable an inline field and reset another field's filter without replacing its other settings:

```kdl
zjstatus {
    tab_pipe "notes" { enabled false; }
    tab_pipe "scratchpads" { include; item_hidden_format ""; }
}
```

#### Delivery and Limits

Use **one authoritative producer per global name or tab field**. Delivery is session-wide and last-writer-wins, not client-private. The producer passively discovers configured scratchpad instances opened by other clients from the shared registry, validated against observed panes, without reconciling or writing registry records during publication. Focus and MRU reflect that producer's client view and history, not a merged multi-client view. Automatic adoption of manually moved cross-tab scratchpads is out of scope; moving a pane does not transfer its scratchpad identity or badge.

Updates follow scratchpad actions, pane/tab changes and configuration changes. New receivers also trigger delayed replay. `zellij-tools::zjstatus::refresh` forces replay; periodic full replay recovers new/restarted receivers and startup races even without state changes. `refresh_ms` is an unquoted positive KDL integer in `1..=4294967295` (`u32`), default `2000` milliseconds. Zero, negatives, overflow, strings and booleans are rejected. Replay is independent of `watch_ms` and requires no external include file.

For the default scratchpad source, an empty rendered result uses the literal `empty_format` fallback after sanitization. If the result is still empty, a **tab output sends a clear**, including on its first publication; a **global output skips the write**, retaining the receiver's previous value. Whitespace is nonempty. A count-only format such as `"{tab_live_count}"` renders `0`, not an empty value, so its wrapper remains visible. Disabling/removing scratchpad global outputs does not guarantee clearing their old receiver values. Pane-status outputs instead publish clears for both kinds, as described below.

Disabling, removing or renaming a tab output clears its previously owned fields on live tabs. Retired tab keys keep replaying clears during the producer's lifetime, **even with all outputs disabled**. Replay stops when there are neither enabled outputs nor retired keys needing clears. Retired keys are dropped when their tab closes or the key is reactivated. Receiver state and producer retirement history are transient: there is no acknowledgement, TTL, persistent ownership history or automatic cleanup after a producer crash/restart.

Tab delivery uses native stable IDs, not positions or pane IDs, and broadcasts every tab's values to all receivers, including bars in inactive tabs:

```text
zjstatus::tab_pipe::42::scratchpads::term+ git-
zjstatus::tab_pipe::42::scratchpads::
zjstatus::pipe::pipe_scratchpads::term
```

The second command clears tab ID `42`'s field; its trailing `::` is required. IDs here are illustrative; obtain actual IDs from `zellij-tools tree`. Tab values preserve embedded `::`; global values replace it with `: :`. Both replace CR/LF with spaces. Data-derived names/titles are escaped rather than interpreted as style markup or recursively expanded placeholders.

#### zjstatus Placeholders

For the default scratchpad source, global pipe `format` supports these placeholders:

| Placeholder                                                                           | Description                                                   |
| ------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| `{current_items}`, `{global_items}`                                                   | Rendered scratchpad items for the active tab or whole session |
| `{current_configured_count}`, `{global_configured_count}`, `{other_configured_count}` | Status entries after include/exclude filters                  |
| `{current_rendered_count}`, `{global_rendered_count}`, `{other_rendered_count}`       | Count entries whose item format renders nonempty              |
| `{current_live_count}`, `{global_live_count}`, `{other_live_count}`                   | Scratchpads with live panes                                   |
| `{current_visible_count}`, `{global_visible_count}`, `{other_visible_count}`          | Visible scratchpad counts                                     |
| `{current_hidden_count}`, `{global_hidden_count}`, `{other_hidden_count}`             | Hidden/suppressed scratchpad counts                           |
| `{current_closed_count}`, `{global_closed_count}`, `{other_closed_count}`             | Configured scratchpads without a live pane                    |
| `{current_focused_name}`, `{global_focused_name}`                                     | Focused scratchpad name                                       |
| `{current_focused_title}`, `{global_focused_title}`                                   | Focused scratchpad title                                      |

`current_*` values describe the producer's active tab. `global_*` values describe the whole session. `other_*` counts are `global` minus `current`, floored at zero. Global items render one representative per configured name, but global counts use each live instance, or one closed entry when a name has no live instances. Thus a name live on two tabs contributes two to global configured/live counts while rendering once in `{global_items}`; global rendered counts also evaluate the per-instance entries.

Tab outputs support `{tab_items}`, `{tab_configured_count}`, `{tab_rendered_count}`, `{tab_live_count}`, `{tab_visible_count}`, `{tab_hidden_count}`, `{tab_closed_count}`, `{tab_focused_name}`, and `{tab_focused_title}`, plus the same `{global_*}` placeholders. `tab_*` always describes the tab whose label is being rendered, including inactive tabs. Known `current_*`/`other_*` placeholders are rejected in tab outputs; `tab_*` placeholders are rejected in global outputs.

Visible means live, floating and not suppressed, even on an inactive tab. Hidden means live but not visible (including tiled panes); absent, exited or held panes count as closed. Focus/MRU are supplementary attributes, not lifecycle states.

Item formats support these placeholders:

| Placeholder             | Description                                       |
| ----------------------- | ------------------------------------------------- |
| `{name}`                | Scratchpad config name                            |
| `{title}`               | Configured title, falling back to the name        |
| `{state}`               | `visible`, `hidden`, or `closed`                  |
| `{icon}`                | Fixed state icon: `●`, `○`, or `×`                |
| `{pane_id}`, `{tab_id}` | Numeric terminal pane ID and native stable tab ID |
| `{tab_position}`        | Zero-based API tab position when available        |
| `{is_focused}`          | `true` or `false`                                 |

Closed tab-local items retain their target `{tab_id}` and `{tab_position}` but have an empty `{pane_id}`. `{tab_position}` preserves the existing zero-based convention, unlike the CLI's 1-based `focus tab` position argument and the receiver's display index.

Item format precedence is: scoped focused override, scoped MRU override, `<scope>_item_<state>_format`, `item_<state>_format`, `<scope>_item_format`, then `item_format`. Focused/MRU overrides exist for `current` and `tab`, not `global`. MRU is the focused scratchpad or, when none is focused, the target of an unnamed toggle using that tab's focus history. No MRU is selected without focus or history. An explicit empty override omits the item; a nonempty focused/MRU override can show an item even when `item_hidden_format ""` would otherwise hide it.

Inside each output, supported keys are `enabled`, `format`, `empty_format`, `include`, `exclude`, `item_format`, `item_visible_format`, `item_hidden_format`, `item_closed_format`, and `item_separator`. Scoped overrides are `<scope>_item_format`, `<scope>_item_visible_format`, `<scope>_item_hidden_format`, `<scope>_item_closed_format`, and `<scope>_item_separator`, using `current`/`global` for global outputs and `tab`/`global` for tab outputs. `current_item_focused_format`/`current_item_mru_format` apply only to global outputs; `tab_item_focused_format`/`tab_item_mru_format` only to tab outputs. Formats and separators are strings; unknown options, duplicate options and unsupported scopes are errors.

#### Pane Status Outputs

Add separate outputs to the same `zjstatus` block to display [pane statuses](#pane-status) without changing scratchpad items:

```kdl
zjstatus {
    pipe "pane_status" {
        source "pane-status"
        format "Status: {current_items} "
        item_format "{title}: {status}"
        current_item_focused_format "#[bold]{title}: {status}"
        item_separator " | "
    }
    tab_pipe "pane_status" {
        source "pane-status"
        // Defaults: format "{tab_items}" and item_format "{status}".
        item_separator " / "
        // Optional exact typed IDs, not scratchpad names or bare numbers:
        // include "terminal_2" "plugin_7"
        // exclude "terminal_3"
    }
}
```

The global pipe shows all eligible statuses in the producer's **active tab**, not just the focused pane. Each tab field shows only statuses belonging to that actual tab, including inactive tabs. Items are sorted by terminal IDs first, then plugin IDs, numerically within each type. Statuses follow panes moved between tabs, unlike scratchpad identities.

Pane-status source options and placeholders:

- `format` defaults to `"{current_items}"` for `pipe` and `"{tab_items}"` for `tab_pipe`. The corresponding count is `{current_rendered_count}` or `{tab_rendered_count}`. There are no session-wide `{global_*}`, `{other_*}`, scratchpad lifecycle counts, or MRU placeholders for this source.
- `item_format` defaults to `"{status}"`. Item placeholders are `{status}`, `{title}` (current pane title), `{pane_id}` (typed canonical ID), `{tab_id}` (native tab ID), and `{is_focused}` (`true`/`false`). Status text and titles are not recursively expanded; a space is inserted after every opening `{` in this untrusted text, while closing `}` is unchanged. Config template placeholders remain unchanged.
- Item precedence is `current_item_focused_format`, `current_item_format`, then `item_format` for global pipes; use `tab_item_focused_format` and `tab_item_format` for tab pipes. The focused override applies only to focused items. An explicitly empty selected item format omits that item.
- `item_separator` defaults to `" "`; `current_item_separator` or `tab_item_separator` overrides it for the matching scope. The option is **not** `separator`.
- `include`/`exclude` match exact canonical typed IDs such as `terminal_2` and `plugin_7`, not `2` or `terminal_02`. Exclude wins. Empty include means all IDs; filtering does not change sort order.
- With no eligible rendered items, the producer uses literal `empty_format` (default `""`) instead of evaluating `format`. Even a prefix or count-only format is hidden by default, rather than displaying a stale label or `0`.

Configure matching receiver fields with **dynamic mode, even when every status uses plain format**: the producer inserts style-isolation markup around interpolations and output. Static mode is not suitable for this source. Merge these settings and placeholders into the existing receiver configuration, retaining existing scratchpad widgets:

```kdl
pipe_pane_status_format "{output}"
pipe_pane_status_rendermode "dynamic"
tab_pipe_pane_status_format " {output}"
tab_pipe_pane_status_rendermode "dynamic"
// Append {pipe_pane_status} to an outer bar format, for example format_right.
// Append {tab_pipe_pane_status} to tab_normal, tab_active, and any variants.
```

Use exactly `"{output}"` for the global receiver format and put decoration/spacing in the producer's `format`, so clearing does not leave stale receiver decoration. Tab fields may use a spacing wrapper as above because cleared tab values hide their wrappers. Tab fields still require the tab-pipe-capable zjstatus build described above.

Dynamic style resets do not mean the same inherited defaults in global and tab rendering; do not rely on the surrounding bar/tab style. Producer item/output templates support styles with explicit inheritance, and the producer restores the trusted template style after each interpolation. Receiver style-wrapper overrides are not guaranteed: prefer receiver format `"{output}"` and place trusted colors in the producer's `item_format`. No raw ANSI is supported. Embedded `::` survives tab delivery but becomes `: :` in global output due to the existing global receiver protocol limitation.

Both empty global and tab pane-status outputs send clears. Disabling, removing, or renaming a previously published pane-status output also retires its old keys and replays clears during the publisher's lifetime. This does not provide persistent cleanup after a crash. Use a **single authoritative configured publisher**: delivery remains session-wide, last-writer-wins, with the publisher's active-tab/focus view. This feature does not add multi-client aggregation or client-private status widgets.

#### Local Dev Example

`nix run .#dev` builds the `b0o/zjstatus` fork's `feat-tab-pipe` branch from the revision pinned in `flake.lock`. The launcher copies the Nix-store artifact to the gitignored `.cache/zjstatus.wasm`, which `dev.kdl` loads by relative path. Run from this repository's root; each launch refreshes the cached artifact. No installed plugin or production config needs replacing.

```sh
# Build just the pinned status bar.
nix build .#zjstatus

# Update the fork pin after changes have been pushed.
nix flake update zjstatus

# Test uncommitted local fork changes without changing the lock file.
nix run .#dev --override-input zjstatus path:/home/boo/git/zjstatus/worktree/feat-tab-pipe -- --session tools-tab-pipes-demo
```

From this repository, build with `nix run .#build`, then launch an isolated session from a terminal outside your active Zellij session with `nix run .#dev -- --session tools-tab-pipes-demo`. The layout starts `dev` and `review`, each with a status bar; scratchpads are not auto-opened.

1. In the first tab's terminal, run `nix run .#run -- scratchpad show dev`, then `nix run .#run -- scratchpad hide dev` from the scratchpad shell. This leaves a hidden `dev` instance.
2. Switch to tab 2 with `Ctrl Space`, then `2`. Run `nix run .#run -- scratchpad show git` there and leave it visible.
3. Both bars should show `dev-` beside the first tab and `git+` beside the second. `Ctrl T`, `Ctrl G`, and `Ctrl B` toggle `dev`, `git`, and `monitor` on the invoking tab. The separate monitor field is visible after showing `monitor` with `Ctrl B`.
4. Hide/show/close these instances and verify their own tab labels change. For the same-name case, also show `dev` on tab 2; it is independent of the hidden `dev` on tab 1.

CLI commands infer the originating pane's tab. Explicit `--tab` values must be native IDs from `tree`, not the layout's first/second positions. These are manual verification instructions, not recorded test outcomes.

The dev configuration also displays pane statuses in the main bar and every tab-label variant. In a dev-session terminal, try:

```sh
nix run .#run -- pane-status set 'Building'
nix run .#run -- pane-status set --format zjstatus '#[fg=yellow,bold]Permission Requested'
nix run .#run -- pane-status clear
```

Before clearing, switch tabs: the original tab's badge should remain, but its status should leave the main bar. Set another status in a second pane to check aggregation. Main-bar items include pane titles; tab badges show only messages. Clearing the last status removes the output and its decoration.

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

## Pane Status

Each typed pane ID (`terminal_N` or `plugin_N`) can own one ephemeral status. Setting replaces its previous value; clearing or setting an empty string removes it. It survives focus changes without expiry, follows pane moves, and is removed when the pane closes or the plugin/session restarts. Statuses are not persisted and do not change the tree or event-stream schemas.

```sh
zellij-tools pane-status set 'Building'
zellij-tools pane-status set --format zjstatus '#[fg=yellow,bold]Permission requested'
zellij-tools pane-status clear
zellij-tools pane-status set --pane-id terminal_2 'Testing'
zellij-tools pane-status clear --pane-id plugin_7
zellij-tools --session other pane-status set --pane-id terminal_2 'Remote build'
```

Both commands accept `--pane-id`, defaulting to the originating pane from `ZELLIJ_PANE_ID`, not whichever pane is focused. A numeric environment value is normalized to `terminal_N`; explicit `--pane-id` must be typed. Missing/invalid environment values require an explicit target. If `--session` differs from `ZELLIJ_SESSION_NAME`, or that environment variable is unset, an explicit typed `--pane-id` is required. IDs are local to the target session; use `zellij-tools tree` to discover them.

`set --format plain|zjstatus` defaults to `plain`. Plain text neutralizes `#[` to `# [`. In both plain and formatted status text, and in pane titles, a space is inserted after every opening `{`; closing `}` is unchanged. For example, `{command_x}` becomes `{ command_x}`. The pinned receiver's click handler searches for original widget tokens in already-expanded output, so widget-looking input cannot safely pass through unchanged: even without recursive rendering, it can shift configured command click hitboxes. This neutralization does not change config template placeholders. `zjstatus` format accepts only this safe markup subset:

- `#[fg=COLOR,bg=COLOR,bold,italic,underscore]`, with any supported directives combined using commas without spaces, plus `#[]` to reset.
- Colors: `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, their `bright_` variants, decimal indices `0..255`, or six-digit `#RRGGBB`. Aliases, `default`, other style directives, and malformed markers are rejected.
- Maximum input size: 4096 UTF-8 bytes; formatted input may contain at most 64 style markers, excluding producer-generated isolation markers.
- Control characters (including newlines, tabs, and ANSI escapes), bidi controls, zero-width formatting characters, soft hyphens, and other unsafe invisible formatting characters are rejected in both formats. Invalid updates leave the existing status unchanged.

The CLI checks arguments, byte length, and control characters locally; the plugin performs authoritative pane and style validation. **CLI success means transport acceptance, not a plugin validation acknowledgement.** Plugin errors are logged by the plugin. Unknown panes and updates sent before pane discovery is ready are rejected, not queued; verify the target and retry after discovery.

Direct pipe equivalents (replace illustrative IDs with live pane IDs):

```sh
zellij pipe --plugin zellij-tools -- 'zellij-tools::pane-status::terminal_2::text'
zellij pipe --plugin zellij-tools --args format=zjstatus -- 'zellij-tools::pane-status::terminal_2::#[fg=green]Ready'
zellij pipe --plugin zellij-tools -- 'zellij-tools::pane-status::terminal_2::'
```

The empty final `::` clears the status and is required. The message is everything after the typed pane ID, so embedded `::` is preserved on input. Pipe argument `format` defaults to `plain`; only `plain` and `zjstatus` are accepted. To display statuses, configure [pane status outputs](#pane-status-outputs).

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
- `MessageAndLaunchOtherPlugins` - Publish scratchpad and pane status to zjstatus

## License

&copy; 2025-2026 Maddison Cohodas

MIT License
