# Contributing

## Local setup

```bash
cargo build
cargo run -p lensmap -- --help
```

## Validation checklist

```bash
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

## Release process

1. Update `CHANGELOG.md`.
2. Bump `workspace.package.version` in `Cargo.toml`.
3. Tag release: `git tag vX.Y.Z && git push origin vX.Y.Z`.
4. GitHub Actions will build and publish binaries.
