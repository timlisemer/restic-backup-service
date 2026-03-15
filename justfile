# Default recipe - show help
default:
    @just --list

# Clean all build artifacts
clean:
    cargo clean

# Full build with tests
build:
    cargo build
    cargo test
    cargo doc --open

# Run the application (pass extra args, e.g. `just run --release`)
run *args:
    cargo run {{args}}

# Run linters, tests, and formatters
check:
    # Run clippy on all targets
    cargo clippy --all-targets
    # Run tests
    cargo test
    # Format Rust code
    cargo fmt
