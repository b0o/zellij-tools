target := "wasm32-wasip1"

build:
    cargo build --target={{target}}

build-release:
    cargo build --release --target={{target}}

fmt:
    dprint fmt
