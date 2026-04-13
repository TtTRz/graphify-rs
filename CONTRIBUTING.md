# Contributing to graphify-rs

Thank you for considering contributing to graphify-rs! This guide will help you get started.

## Getting Started

```bash
git clone https://github.com/TtTRz/graphify-rs.git
cd graphify-rs
cargo build --workspace
cargo test --workspace
```

### Prerequisites

- Rust 1.85+ (install via [rustup](https://rustup.rs/))
- Git (for temporal analysis features)
- No external dependencies — all 14 crates are self-contained

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

## Architecture Overview

graphify-rs is organized as a 14-crate Cargo workspace. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full design.

Key crates to know:

| Crate | What it does | When to modify |
|-------|-------------|----------------|
| `graphify-core` | Data models, graph structure | Adding new node/edge types |
| `graphify-extract` | AST parsing (21 languages) | Adding language support |
| `graphify-analyze` | PageRank, cycles, embeddings | Adding analysis algorithms |
| `graphify-serve` | MCP server (15 tools) | Adding query capabilities |
| `graphify-cluster` | Leiden community detection | Improving clustering |
| `graphify-export` | 9 output formats | Adding export formats |

## Code Style

- Run `cargo fmt` before committing
- All clippy warnings must be resolved (`-D warnings`)
- Public APIs should have `///` doc comments
- Use `thiserror` for library error types, `anyhow` for the binary crate
- Each crate should have unit tests in `#[cfg(test)]` modules or `tests/` directory

## Testing

### Running tests

```bash
cargo test --workspace              # all 378+ tests
cargo test -p graphify-extract      # single crate
cargo test -- pagerank              # filter by name
```

### Test expectations

- **New features**: Must include at least 2 tests (happy path + edge case)
- **Bug fixes**: Must include a regression test
- **New languages**: Must include AST extraction test + cross-file resolution test
- **New MCP tools**: Must update `test_tools_list` assertion count

### Test organization

- Unit tests: `#[cfg(test)] mod tests` in source files (for private function access)
- Integration tests: `crates/*/tests/*.rs` (for public API testing)
- E2E tests: `tests/integration/` (full pipeline validation)

## Adding a New Language

To add tree-sitter support for a new language:

1. Add the grammar crate to `crates/graphify-extract/Cargo.toml`
2. Add a `ts_config_<lang>()` function in `treesitter.rs`
3. Register the language in the `DISPATCH` table in `lib.rs`
4. Add extraction test in `tests/ast_extract.rs`
5. Add cross-file resolution in `resolve_cross_file_imports()` if applicable

For regex-only fallback, add a `LanguageConfig` in `lang_config.rs`.

## Pull Request Guidelines

- Keep PRs focused — one feature or fix per PR
- Include tests for new functionality
- Update CHANGELOG.md under `## [Unreleased]`
- CI must pass before merge
- PRs are reviewed by maintainers within 3 business days

### PR checklist

```
- [ ] `cargo fmt --all` passes
- [ ] `cargo clippy --workspace -- -D warnings` passes
- [ ] `cargo test --workspace` passes (all 378+ tests)
- [ ] New code has tests
- [ ] CHANGELOG.md updated
- [ ] Documentation updated (if API changed)
```

## Release Process

Releases follow [Semantic Versioning](https://semver.org/):

- **Patch** (0.4.x): Bug fixes, documentation updates
- **Minor** (0.x.0): New features, new languages, new MCP tools
- **Major** (x.0.0): Breaking API changes

Release steps:
1. Update version in root `Cargo.toml` (workspace version propagates to all 14 crates)
2. Update `CHANGELOG.md` — move `[Unreleased]` items to new version
3. Run full test suite
4. Publish in dependency order (5 tiers, 15 crates)
5. Create GitHub release with tag

## Reporting Issues

- Use GitHub Issues for bug reports and feature requests
- Include reproduction steps, expected vs actual behavior
- For extraction bugs, include a minimal code sample
- For security issues, see [SECURITY.md](SECURITY.md)
