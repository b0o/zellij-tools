## Project

This is zellij-tools, a Zellij plugin plus a companion CLI.

- Plugin crate: receives `zellij pipe` messages and controls panes/tabs from inside Zellij.
- CLI crate (`cli/`): sends pipe messages, streams events, and provides one-shot tree/focus helpers.
- Primary transport: pipe payloads in the form `zellij-tools::event::arg1::arg2::...`.

## The pipe lifecycle method

Plugins may listen to pipes by implementing the pipe lifecycle method. This method is called every time a message is sent over a pipe to this plugin (whether it's broadcast to all plugins or specifically directed at this one). It receives a PipeMessage containing the source of the pipe (CLI, another plugin or a keybinding), as well as information about said source (the plugin id or the CLI pipe id). The PipeMessage also contains the name of the pipe (explicitly provided by the user or a random UUID assigned by Zellij), its payload if it has one, its arguments and whether it is private or not (a private message is one directed specifically at this plugin rather than broadcast to all plugins).

Similar to the update method, the pipe lifecycle method returns a bool, true if it would like to render itself, in which case the render function will be called as normal.

Here's a small Rust example:

```rust
fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
    let mut should_render = false;
    match pipe_message.source {
        PipeSource::Cli(input_pipe_id) => {
            if let Some(payload) = pipe_message.payload {
                self.messages_from_cli.push(payload);
                should_render = true;
            }
            if self.paused {
                // backpressure, this will pause data from the CLI pipeline until the unblock_cli_pipe_input method will be called for this id
                // from this or another plugin
                block_cli_pipe_input(&input_pipe_id);
            }
            if self.should_print_to_cli_stdout {
                // this can happen anywhere, anytime, from multiple plugins and is not tied to data from STDIN
                // as long as the pipe is open, plugins with its ID can print arbitrary data to its STDOUT side, even if the input side is blocked
                cli_pipe_output(input_pipe_id, &payload);
            }
        }
        PipeSource::Plugin(source_plugin_id) => {
            // pipes can also arrive from other plugins
        }
    }
    should_render
}
```

## Architecture

### Modules

- `src/main.rs` - Plugin entrypoint, permission/event subscriptions, pipe routing, config watch loop
- `src/message.rs` - Parses payloads and validates plugin prefix/format
- `src/focus.rs` - `focus-tab` target parsing (`position` and `id` forms)
- `src/scratchpad/` - Scratchpad actions, config parsing, persistence/reconciliation
- `src/pane_status.rs` - Ephemeral typed-pane statuses, validation, escaping, tab snapshots
- `src/zjstatus.rs` - Scratchpad/pane-status output config and rendering
- `src/zjstatus_runtime.rs` - Session-wide publication, receiver discovery, replay and retirement
- `src/tree.rs` - Session tree snapshot serialization (tabs/panes/tab IDs)
- `src/events.rs` - Subscription state machine and pane/tab event diffing
- `src/config.rs` - WASI-safe env/config path resolution via `/host/proc/self/environ`
- `cli/src/main.rs` - User CLI (`focus`, `scratchpad`, `pane-status`, `subscribe`, `tree`) and heartbeat-driven stream client

### Key Features

**Scratchpads**: Floating terminal panes that can be toggled on/off. They follow you across tabs and persist state. Scratchpads support configurable coordinates (`x`, `y`, `width`, `height`), origin anchoring (`center`, `top`, `bottom`, etc.), custom titles, and working directories.

**External Config Hot-Reload**: Scratchpad definitions can be loaded from an external KDL file that is polled for changes. Inline and external configs are merged; external entries override inline entries with the same scratchpad name.

**Focus Helpers**: Focus pane by typed pane ID (`terminal_N` / `plugin_N`) and focus tab by position or Zellij's native tab ID.

**Event Subscription + Tree API**: Subscribe to pane/tab lifecycle/focus events (compact or full mode) and query a full JSON session tree snapshot.

## Pipe Message Conventions

- Canonical payload format: `zellij-tools::event::arg1::arg2::...`
- Messages for other plugins or invalid keybind payloads are ignored (`WrongPlugin`/`InvalidFormat`).
- Unknown events or invalid arguments are surfaced as plugin errors.

### Supported Events

- `focus-pane::<pane_id>` where `pane_id` parses as Zellij `PaneId` (eg `terminal_2`, `plugin_7`)
- `focus-tab::<position>` (short form)
- `focus-tab::position::<n>`
- `focus-tab::id::<tab_id>`
- `scratchpad::toggle[::<name>]`
- `scratchpad::show::<name>`
- `scratchpad::hide::<name>`
- `scratchpad::close::<name>`
- `pane-status::<typed_pane_id>::<text>` (empty text clears; preserve embedded `::`; pipe arg `format=plain|zjstatus`, default `plain`)
- `subscribe` or `subscribe::full` (CLI pipes only)
- `unsubscribe::<pipe_id>`
- `tree` (CLI pipes only)

`subscribe` now uses a same-pipe init handshake:

1. CLI opens the pipe with `zellij-tools::subscribe` (or `::full`)
2. Plugin registers the subscriber as pending and emits `Ack`
3. CLI writes one raw JSON init line to the same pipe stdin
4. Plugin emits `InitAck` on success or `InitError` on failure
5. Pane/tab streaming starts only after `InitAck`

## Pane Status Notes

- One status per typed `terminal_N`/`plugin_N`; replace/clear explicitly, prune on authoritative pane closure, lose on plugin/session restart. No persistence, focus clearing, expiry, or tree/event schema changes. Moves follow the actual pane/tab.
- CLI `pane-status set <text> [--format plain|zjstatus]` and `clear` accept typed `--pane-id`, otherwise normalize originating `ZELLIJ_PANE_ID` (numeric means terminal). Cross-session `--session` requires an explicit ID when it differs from `ZELLIJ_SESSION_NAME` or that variable is unset.
- CLI success is transport acceptance, not plugin validation acknowledgement. Plugin logs rejection; unknown panes/discovery-not-ready updates are not queued and need retry after discovery. Invalid updates preserve existing state.
- Input limits: 4096 UTF-8 bytes, 64 formatted style markers. Reject controls/ANSI, bidi and unsafe invisible formatting characters. Plain `#[` becomes `# [`. A space is inserted after every opening `{` in plain/formatted status text and titles (e.g. `{command_x}` becomes `{ command_x}`); closing `}` and config template placeholders are unchanged. The pinned receiver's click handler searches original widget tokens in expanded output, so unchanged `{command_x}` input can shift configured command click hitboxes even without recursive rendering. Formatted styles allow only `fg`/`bg` (eight named colors, `bright_` variants, decimal 0..255, `#RRGGBB`), `bold`, `italic`, `underscore`, and empty resets.
- `source "pane-status"` on `pipe`/`tab_pipe` opts in; default source remains `"scratchpad"`. Default item is `{status}`, output `{current_items}`/`{tab_items}`, separator `item_separator " "` (not `separator`). Include/exclude match canonical typed IDs exactly; terminals precede plugins, numeric order, scoped to actual tabs. Global pipe means all eligible active-tab statuses, not just focus.
- Item placeholders: `{title}`, `{status}`, `{pane_id}`, `{tab_id}`, `{is_focused}`. Precedence: scoped focused override, scoped item format, item_format; scopes are current/tab. Counts are `{current_rendered_count}`/`{tab_rendered_count}`. No scratchpad lifecycle/MRU/global aggregation. No rendered items uses literal `empty_format` (default empty), hiding even output prefixes/counts.
- Receiver MUST use dynamic mode even for plain statuses: producer adds isolation markup and restores trusted template style after interpolation. Producer item/output styles support explicit inheritance; resets are not equivalent inherited defaults in global/tab rendering. Receiver style-wrapper overrides are not guaranteed: recommend receiver format `{output}` and trusted colors in producer `item_format`. Global `{output}` avoids stale decoration; tab spacing wrappers hide on clear. No raw ANSI. Tab output preserves `::`; global replaces it with `: :` for the existing receiver limitation.
- Pane-status globals and tabs publish empty clears and replay retired keys during publisher lifetime; scratchpad global empty-skip behavior is unchanged. Use one authoritative configured publisher: session-wide, last-writer-wins delivery, not multi-client aggregation or client-private output.

## Event Streaming Notes

- Subscribe replies with an `Ack` event first so the CLI can confirm connection.
- Subscribers start in pending-init state; no pane/tab stream events are emitted before init succeeds.
- Init payload is raw JSON over the same pipe stdin (not a second `zellij pipe` message).
- Init JSON supports `full`, `events`, `pane_ids`, and `tab_ids` filter fields.
- Plugin emits newline-delimited JSON to CLI pipes (`cli_pipe_output(..."\n")`).
- Empty payloads from CLI are treated as heartbeats for subscriber liveness.
- Stale subscribers are pruned after missed heartbeat ticks.
- Compact mode emits minimal event objects; full mode enriches pane events with fields like `title`, `terminal_command`, `plugin_url`, and suppression/floating flags.

### CLI Subscribe Filter Flags

- `--event` filters by canonical event names (eg `PaneFocused`, `TabMoved`)
- `--pane-id` filters terminal pane IDs
- `--plugin-pane-id` filters plugin pane IDs
- `--tab-id` filters tab IDs
- CLI converts pane ID flags to typed IDs (`terminal_N`, `plugin_N`) in init JSON

### How External Config Works

1. On plugin load, if `include` option is set, store the raw path
2. Request `FullHdAccess` permission to access the filesystem
3. On permission granted, mount `/` to `/host` via `change_host_folder("/")`
4. On `HostFolderChanged`, read environment variables from `/host/proc/self/environ`
5. Resolve the include path using `ZELLIJ_CONFIG_DIR`, `XDG_CONFIG_HOME`, or `HOME`
6. Read the external config file from `/host/<resolved_path>`
7. Start a timer to poll for changes (default: 2000ms)

If both inline and external scratchpads exist, inline config is parsed first, then external config is merged on top (external wins on key collision).

`watch_ms` behavior:

- Missing: defaults to `2000` when `include` is set
- `"0"` or `"false"`: disables polling
- Otherwise: parsed as milliseconds

### WASI Sandbox Notes

Zellij plugins run in a WASI sandbox with limited filesystem access:

- `/host` - Mapped to a host directory via `change_host_folder()`
- `/data` - Plugin data directory (persists across plugin instances)
- `/tmp` - Temporary directory

Environment variables are NOT directly accessible. Read them from `/host/proc/self/environ` after mounting `/` to `/host`.

Include path resolution order for relative includes:

1. `ZELLIJ_CONFIG_DIR`
2. `$XDG_CONFIG_HOME/zellij`
3. `$HOME/.config/zellij`
4. `/etc/zellij` (fallback)

## Permissions and Events

Plugin requests:

- `ReadApplicationState`
- `ChangeApplicationState`
- `RunCommands`
- `ReadCliPipes`
- `FullHdAccess`

Plugin subscribes to:

- `PaneUpdate`
- `TabUpdate`
- `HostFolderChanged`
- `FailedToChangeHostFolder`
- `PermissionRequestResult`
- `Timer`

## Building

- Reproducible Nix commands:
  - `nix build .#default` - Build plugin wasm (`wasm32-wasip1`)
  - `nix build .#cli` - Build CLI binary
  - `nix flake check` - Build flake checks for plugin, CLI, and clippy outputs
- Local development apps, backed by Cargo incremental builds:
  - `nix run .#build-plugin` - Build plugin wasm (`wasm32-wasip1`)
  - `nix run .#build-cli` - Build CLI binary
  - `nix run .#build` - Build plugin and CLI
  - `nix run .#build-plugin-release` - Build plugin wasm in release mode
  - `nix run .#build-cli-release` - Build CLI binary in release mode
  - `nix run .#build-release` - Build plugin and CLI in release mode
  - `nix run .#test` - Run library tests
  - `nix run .#check` - `dprint check` + clippy (`-D warnings`) for plugin and CLI
  - `nix run .#fmt` - Format with `dprint` and `rustfmt`
  - `nix run .#ci` - Run `nix flake check`
  - `nix run .#dev -- <zellij args>` - Run Zellij from upstream main with `dev.kdl`
  - `nix run .#run -- <args>` - Run the CLI with Cargo
  - `nix run .#run-release -- <args>` - Run the CLI in release mode with Cargo

CI/lint conventions:

- Rust toolchain comes from `rust-toolchain.toml`
- Clippy warnings are treated as errors
- Workspace contains two crates: plugin root and `cli/`

Release conventions:

- Do not create GitHub releases manually. Push the release tag only; the GitHub release workflow creates the release.
- Do not build or upload release assets manually. The GitHub release workflow builds and attaches platform assets.
- For a patch release, commit code fixes first, then make a separate version commit that bumps both crate manifests (`Cargo.toml`, `cli/Cargo.toml`) and the corresponding local package entries in `Cargo.lock`.
- Tag the version commit as `vX.Y.Z`, push `main`, push the tag, and verify the tag-triggered `Release` GitHub Actions workflow succeeds.
- Do not replace workflow-generated assets with locally-built artifacts.
