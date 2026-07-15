# Contributing to Conductor

Thank you for your interest in contributing! This document covers the workflow
and conventions for the project.

---

## Development Setup

```bash
git clone https://github.com/2mes4/conductor.git
cd conductor
cp .env.example .env

# Start PostgreSQL
docker compose up -d

# Build & test
cargo build
cargo test
```

### Prerequisites

- Rust 1.75+ (`rustup`)
- PostgreSQL 16+ (or use `docker compose up -d`)
- [OpenCode](https://opencode.ai) CLI (for integration testing)

---

## Code Style

- Format with `cargo fmt` (config in [`rustfmt.toml`](rustfmt.toml))
- Lint with `cargo clippy -- -D warnings`
- Use `snake_case` for functions/variables, `PascalCase` for types
- Prefer `thiserror` for library errors, `anyhow` for application-level
- Document all public items with `///` doc comments

---

## Git Workflow

1. **Fork** the repository and create a branch:
   ```bash
   git checkout -b feature/my-feature
   ```
2. **Commit** with conventional commits:
   - `feat:` new feature
   - `fix:` bug fix
   `docs:` documentation
   - `refactor:` code restructuring
   - `test:` tests
   - `chore:` tooling, deps
3. **Push** and open a Pull Request against `main`.

---

## Pull Request Checklist

- [ ] Code is formatted (`cargo fmt`)
- [ ] Lints pass (`cargo clippy -- -D warnings`)
- [ ] Tests pass (`cargo test`)
- [ ] Public API is documented
- [ ] Commit messages follow conventional commits
- [ ] No secrets or credentials in code

---

## Architecture Awareness

Before contributing, please read [ARCHITECTURE.md](ARCHITECTURE.md) to
understand the four-layer design and how your change fits into the system.

Key principles:
- **Never shell out to Git** — use `git2-rs`
- **Strict schemas** — `deny_unknown_fields` on all manifests
- **Amnesic execution** — the agent process holds no state
- **Deterministic teardown** — every session cleans up identically

---

## Reporting Issues

- Use the issue templates for [bugs](.github/ISSUE_TEMPLATE/bug_report.md) and
  [features](.github/ISSUE_TEMPLATE/feature_request.md)
- Include Rust version, OS, and relevant logs
- For security issues, see [SECURITY](#) (TODO)

## License

By contributing, you agree that your contributions will be licensed under the
[MIT License](LICENSE).
