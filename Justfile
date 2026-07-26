# rat maintainer entrypoints. Run `just --list` for grouped discovery.
# Rust nightly is used for formatting only; everything else runs on stable.

# Recipes are written for a POSIX shell. Git for Windows bundles one but does
# not put it on PATH, so name it explicitly.
set windows-shell := ["C:/Program Files/Git/bin/sh.exe", "-cu"]

# List available recipes.
[group('help')]
default:
    @just --list

# Run all tests.
[group('core')]
test *args:
    cargo +stable nextest run --no-tests pass {{ args }}

# Build (debug).
[group('core')]
build *args:
    cargo +stable build {{ args }}

# Build an optimized binary.
[group('core')]
release *args:
    cargo +stable build --release {{ args }}

# Run the CLI.
[group('core')]
run *args:
    cargo +stable run --bin rat -- {{ args }}

# Type-check all targets without the full clippy/fmt gate (used by the
# Windows CI leg, which runs no tests yet).
[group('core')]
check-types:
    cargo +stable clippy --all-targets --all-features -- -D warnings

# Run Rust formatting checks and Clippy across all targets and features.
[group('quality')]
lint: fmt-check
    cargo +stable clippy --all-targets --all-features -- -D warnings

# Run clippy with auto-fix.
[group('quality')]
fix *args: fmt
    cargo +stable clippy --fix --all-targets --all-features --allow-dirty --allow-staged -- -D warnings {{ args }}

# Format code.
[group('quality')]
fmt *args:
    cargo +nightly fmt --all {{ args }}

# Check Rust formatting without writing files.
[group('quality')]
fmt-check:
    cargo +nightly fmt --all -- --check

# Check conventional commits in the selected range.
[group('quality')]
commit-check range='origin/main..HEAD':
    cog check "{{ range }}"

# Run the complete gate: commit check, build, lint, and tests.
[group('quality')]
check: commit-check build lint test

# Install git hooks (commit-msg and pre-push validation via cocogitto).
[group('maintenance')]
setup-hooks:
    cog install-hook --all --overwrite
