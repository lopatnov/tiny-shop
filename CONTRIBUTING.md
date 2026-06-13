# Contributing to tiny-shop

Thanks for your interest in tiny-shop! This is currently a small, early-stage (Phase 1a) solo
project, so this guide is intentionally short.

## License

The source is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE) — free to view
and use for noncommercial purposes. Any commercial use requires a separate license; see
[COMMERCIAL.md](COMMERCIAL.md) for details. By contributing, you agree that your contributions
are provided under the same terms.

## Before you start

For anything beyond a small fix (new features, behavior changes), please open an issue first to
discuss the change. This helps avoid wasted effort on work that may not fit the current roadmap
or architecture.

## Development workflow

- `main` is protected — all changes go through a pull request with passing CI. Direct commits to
  `main` are not accepted.
- Create a feature branch, make your changes, and open a PR against `main`.

## Build, test, lint, format

Before opening a PR, make sure the following pass locally (see [README.md](README.md) for more
details):

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

## Pull requests

- Keep PRs focused and reasonably small.
- Update documentation (`README.md`, `CHANGELOG.md`) when your change affects behavior, the
  build/run process, or the API/CLI surface.
- CI must be green before merge.

## Reporting bugs and security issues

- For regular bugs, open a GitHub issue with steps to reproduce.
- For security vulnerabilities, **do not** open a public issue — see [SECURITY.md](SECURITY.md).

## Code of Conduct

This project follows the [Code of Conduct](CODE_OF_CONDUCT.md). Please be respectful and
constructive in all interactions.
