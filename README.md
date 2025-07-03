# zellij-tools

A [Zellij](https://github.com/zellij-org/zellij) plugin that adds a few handy utilities.

## Installation

```kdl
plugins {
    zellij-tools location="https://github.com/b0o/zellij-tools/releases/latest/download/zellij-tools.wasm"
}

load_plugins {
    // Load at startup
    zellij-tools
}
```

## Actions

### Focus pane

Focuses a pane by ID. You can get the pane ID for the current pane from the `$ZELLIJ_PANE_ID` environment variable.

```kdl
zellij-tools::focus-pane::<pane_id>
```

Example:

```sh
zellij pipe "zellij-tools::focus-pane::2"
```

## License

&copy; 2025 Maddison Hellstrom

MIT License
