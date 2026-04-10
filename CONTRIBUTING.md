# Contributing to graphify-rs

Thank you for considering contributing to graphify-rs!

## Getting Started

```bash
git clone https://github.com/TtTRz/graphify-rs.git
cd graphify-rs
cargo build --workspace
cargo test --workspace
```

## Development Workflow

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/my-feature`
3. Make your changes
4. Ensure all checks pass:
   ```bash
   cargo fmt --all
   cargo clippy --workspace -- -D warnings
   cargo test --workspace
   ```
5. Commit with a descriptive message
6. Push and open a Pull Request

## Code Style

- Run `cargo fmt` before committing
- All clippy warnings must be resolved (`-D warnings`)
- Public APIs should have `///` doc comments
- Use `thiserror` for library error types, `anyhow` for the binary crate
- Each crate should have unit tests in `#[cfg(test)]` modules

## Adding a New Language

To add tree-sitter support for a new language:

1. Add the grammar crate to `crates/graphify-extract/Cargo.toml`
2. Add a `ts_config_<lang>()` function in `treesitter.rs`
3. Register the language in the `DISPATCH` table in `lib.rs`
4. Add tests in the extract crate

For regex-only fallback, add a `LanguageConfig` in `lang_config.rs`.

## Reporting Issues

- Use GitHub Issues for bug reports and feature requests
- Include reproduction steps, expected vs actual behavior
- For extraction bugs, include a minimal code sample

## Pull Request Guidelines

- Keep PRs focused — one feature or fix per PR
- Include tests for new functionality
- Update CHANGELOG.md under `## [Unreleased]`
- CI must pass before merge
