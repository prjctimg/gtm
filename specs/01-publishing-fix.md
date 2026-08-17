## Spec: crates.io Publishing Fix

### Problem
Publishing failed because `readme.workspace = true` points to a path outside
the package directory, which crates.io rejects.

### Solution
Replace `readme.workspace = true` with `readme = "README.md"` in all five
publishable crate manifests:
- `gtm/Cargo.toml`
- `gtm-core/Cargo.toml`
- `gtm-audio/Cargo.toml`
- `gtm-mpris/Cargo.toml`
- `gtmd/Cargo.toml`

Also remove the `## Development` section from the root `README.md` (content
that belongs only in `CONTRIBUTING.md`).

### Verification
- `cargo publish --dry-run` for each crate (CI environment)
- `cargo check` compiles clean
