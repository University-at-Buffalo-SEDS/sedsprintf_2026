# Contributing

Thank you for contributing to this repository. This document explains the preferred workflow, local setup, formatting
and testing rules, and how to submit changes.

## Table of Contents

- Getting started
- Branching & pull requests
- Development setup (macOS)
- Formatting \& linting
- Tests
- Commit messages
- Code review \& CI
- Reporting issues

## Getting started

- Fork the repository and clone your fork.
- Ensure you have `origin` and the main project as `upstream` (if desired).
- The primary development branch for contributions is `dev`. Open pull requests against `dev`.

## Branching \& pull requests

- Create a feature branch from `dev`:  
  `git checkout dev && git pull origin dev && git checkout -b feat/short-description`
- Keep changes focused and small. One logical change per PR.
- Push to your fork and open a PR from your branch into `dev`.
- Include a clear description, motivation, and tests or screenshots when applicable.

## Development setup (macOS)

- Install Rust toolchain: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- Install Python (3.8+ recommended) and a virtual environment:
    - `python3 -m venv .venv && source .venv/bin/activate`
    - `pip install -r requirements.txt` (if present)
- Build and run Rust tests: `cargo test`
- Build any C components as documented in their folders (e.g., `make`).

## Formatting \& linting

- Rust:
    - Format: `cargo fmt --all`
    - Lint: `cargo clippy --all-targets --all-features -- -D warnings`
- Python:
    - Format with Black: `python -m black .`
    - Lint with ruff/flake8 as configured: `python -m ruff .` or `flake8 .`
- Run linters/formatters before opening a PR.

## Tests

- Run tests: `python3 build.py test`
- Include tests for bug fixes and new features. CI must pass before merging.

## Commit messages

- Use clear, imperative messages. Example: `feat(parser): add support for X` or `fix(build): correct linkage when Y`.
- Small, focused commits are easier to review. Squash or rebase as needed before merge.

## Code review \& CI

- All PRs should have passing CI checks and at least one approving review.
- Address review comments promptly. Keep the conversation polite and constructive.
- Maintainers may request changes or adjust scope for consistency.

## Reporting issues

- Open an issue with a descriptive title and reproduction steps.
- Provide environment details (OS, Rust/Python versions) and minimal repro if possible.

## License \& acknowledgements

- By contributing you agree that your contributions will be licensed under the project's existing license.
- Credit third-party code and follow upstream licenses for dependencies.

Thank you for improving the project.
