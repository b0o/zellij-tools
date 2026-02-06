target := "wasm32-wasip1"

build:
    cargo build --target={{target}}

build-release:
    cargo build --release --target={{target}}

build-cli:
    cargo build -p zellij-tools-cli

build-cli-release:
    cargo build --release -p zellij-tools-cli

build-all: build build-cli

build-all-release: build-release build-cli-release

fmt:
    dprint fmt

test:
    cargo test --lib
