# Contributing to Paperboat

## Development Setup

### Prerequisites

- Rust stable (1.75+)
- Git

### Getting Started

```bash
git clone https://github.com/dbmrq/paperboat
cd paperboat
./scripts/install-hooks.sh  # Install pre-commit hooks
cargo build --all-features
```

## Code Quality Tools

Paperboat uses comprehensive quality tooling. This guide documents all tools and how to run them.

### Fast Checks (Pre-commit)

These run automatically on every commit via the pre-commit hook:

| Tool | Command | Purpose |
|------|---------|---------|
| **rustfmt** | `cargo fmt --check` | Code formatting |
| **clippy** | `cargo clippy --all-features -- -D warnings` | Linting |
| **machete** | `cargo machete` | Unused dependency detection |

To run manually:
```bash
cargo fmt              # Fix formatting
cargo fmt -- --check   # Check only
cargo clippy --all-features -- -D warnings
cargo machete
```

### Security & Dependency Checks

| Tool | Command | Purpose |
|------|---------|---------|
| **cargo-audit** | `cargo audit` | Security vulnerability scan |
| **cargo-deny** | `cargo deny check all` | License, ban, advisory, source checks |
| **cargo-outdated** | `cargo outdated` | Check for outdated dependencies |
| **cargo-udeps** | `cargo +nightly udeps` | Thorough unused dep detection |

```bash
# Install tools
cargo install cargo-audit cargo-deny cargo-outdated
cargo install cargo-udeps  # requires nightly

# Run checks
cargo audit
cargo deny check all --all-features
cargo outdated
cargo +nightly udeps --all-features
```

### Code Coverage

| Tool | Command | Purpose |
|------|---------|---------|
| **cargo-llvm-cov** | `cargo llvm-cov` | Coverage report (llvm-based) |

```bash
# Install
cargo install cargo-llvm-cov

# Generate report
cargo llvm-cov --all-features              # Summary
cargo llvm-cov --all-features --html       # HTML report in target/llvm-cov/html/
cargo llvm-cov --all-features --lcov       # LCOV format
cargo llvm-cov --all-features --open       # Open HTML report in browser
```

Coverage is reported to [Codecov](https://codecov.io/gh/dbmrq/paperboat) on every PR.

### Analysis Tools

These tools are useful for specific analysis tasks:

| Tool | Command | Purpose |
|------|---------|---------|
| **cargo-bloat** | `cargo bloat --release` | Binary size analysis |

```bash
# Install
cargo install cargo-bloat

# Analyze binary size
cargo bloat --release                      # By crate
cargo bloat --release --crates             # Summary by crate
cargo bloat --release -n 20                # Top 20 functions
cargo bloat --release --filter paperboat   # Only this crate's code
```

### Testing

```bash
cargo test                          # Run all tests
cargo test --all-features           # With all features enabled
cargo test --no-default-features    # Without optional features
cargo test -- --nocapture           # Show println! output
```

## Configuration Files

| File | Purpose |
|------|---------|
| `Cargo.toml` | Clippy lints in `[lints.clippy]`, unsafe_code forbidden in `[lints.rust]` |
| `rustfmt.toml` | Code formatting rules |
| `deny.toml` | cargo-deny configuration (licenses, bans, advisories) |
| `codecov.yml` | Codecov configuration |
| `scripts/pre-commit` | Pre-commit hook script |

## CI Workflows

| Workflow | Trigger | Purpose |
|----------|---------|---------|
| `ci.yml` | Push/PR | Full CI: fmt, clippy, build, test, security |
| `coverage.yml` | Push/PR | Code coverage to Codecov |
| `security.yml` | Weekly + manual | Deep security checks, outdated deps |
| `release.yml` | Tags | Binary releases |
| `test-windows.yml` | Manual | Thorough Windows testing (including E2E) |

### CI Jobs (ci.yml)

The main CI workflow runs these jobs with optimized caching and fail-fast behavior:

**Fast checks (run first):**
- `fmt` - Code formatting check
- `lockfile` - Cargo.lock consistency
- `unused-deps` - Unused dependency detection
- `code-quality` - TODO/FIXME scan, large file check (informational, doesn't block CI)

**Security checks:**
- `security-audit` - Vulnerability scan (cargo-audit via rustsec)
- `deny` - License, ban, advisory, source checks (cargo-deny)

**Build & lint (waits for fast checks):**
- `clippy` - Linting (all feature combinations, `-D warnings`)
- `build` - Debug and release builds (all feature combinations)
- `build-windows` - Windows cross-compilation
- `docs` - Documentation build (`-D warnings`)

**Tests:**
- `test` - Unit/integration tests (all features, no features)
- `test-windows` - Windows test suite
- `test-install-script` - Installer verification (Unix + Windows)

**Status:**
- `ci-success` - Final status check for branch protection

## Submitting Changes

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/my-feature`
3. Make changes and ensure all checks pass
4. Commit with a clear message
5. Push and open a PR

### Before Submitting

Run all checks locally:
```bash
cargo fmt -- --check
cargo clippy --all-features -- -D warnings
cargo test --all-features
cargo machete
cargo deny check all --all-features
```

Or use the pre-commit hook (auto-runs on commit).

