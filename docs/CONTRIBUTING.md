# Contributing to DEFS

Thank you for your interest in contributing to DEFS! This document will help you get started.

## Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/YOUR_USERNAME/DEFS.git`
3. Build the project:
   ```bash
   cd DEFS/engine
   cargo build --all --features std
   ```
4. Run tests:
   ```bash
   cargo test --all --features std
   ```

## Development Workflow

- Create a feature branch: `git checkout -b feature/my-feature`
- Make your changes
- Ensure tests pass: `cargo test --all --features std`
- Format your code: `cargo fmt -p defs-core -p defs-cli`
- Commit with a clear message
- Open a Pull Request against `main`

## Commit Message Format

```
type: short description

Longer explanation if needed.

Fixes #123
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`

## Areas Needing Help

- **Performance benchmarks** — More real-world workload comparisons
- **FUSE hardening** — Extended attributes, readdirplus, lock operations
- **Documentation** — More examples and tutorials
- **Platform support** — Windows port, BSD testing

## Questions?

Open a [GitHub Discussion](https://github.com/taylor-oclaw/DEFS/discussions) or reach out via the project issues.
