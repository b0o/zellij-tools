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
- `src/tree.rs` - Session tree snapshot serialization (tabs/panes/tab IDs)
- `src/events.rs` - Subscription state machine and pane/tab event diffing
- `src/config.rs` - WASI-safe env/config path resolution via `/host/proc/self/environ`
- `cli/src/main.rs` - User CLI (`focus`, `scratchpad`, `subscribe`, `tree`) and heartbeat-driven stream client

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
- `subscribe` or `subscribe::full` (CLI pipes only)
- `unsubscribe::<pipe_id>`
- `tree` (CLI pipes only)

`subscribe` now uses a same-pipe init handshake:

1. CLI opens the pipe with `zellij-tools::subscribe` (or `::full`)
2. Plugin registers the subscriber as pending and emits `Ack`
3. CLI writes one raw JSON init line to the same pipe stdin
4. Plugin emits `InitAck` on success or `InitError` on failure
5. Pane/tab streaming starts only after `InitAck`

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
- `--tab-id` filters stable tab IDs
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

- Preferred commands (from `Justfile`):
  - `just build-release` - Build plugin wasm (`wasm32-wasip1`)
  - `just build-cli-release` - Build CLI binary
  - `just build-all-release` - Build plugin + CLI release artifacts
  - `just test` - Run library tests
  - `just check` - `dprint check` + clippy (`-D warnings`) for plugin and CLI
  - `just fmt` - Format with `dprint`

CI/lint conventions:

- Rust toolchain tracks `rust-version = 1.84`
- Clippy warnings are treated as errors
- Workspace contains two crates: plugin root and `cli/`
