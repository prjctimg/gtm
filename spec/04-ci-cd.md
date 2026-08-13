# Spec 04: CI/CD — Android in Release Pipeline

## Summary

Extend `.github/workflows/release.yml` to build, package, and release Android/Termux binaries alongside existing Linux and macOS builds. Add an `aarch64-android` matrix entry that cross-compiles with `cargo-ndk`, runs `termux-elf-cleaner`, and produces `.tar.gz` archives and `.deb` packages.

## Current pipeline structure

```
tag job → derive version, ensure tag
  │
  ▼
build job → matrix: [ubuntu-latest, ubuntu-24.04-arm, macos-latest]
  │         cargo build, manpages, completions, archives, .deb
  │
  ▼
release job → create GitHub Release with all artifacts
```

## Proposed change

Add `aarch64-android` to the build matrix:

```
build job → matrix:
  include:
    - os: ubuntu-latest,    platform: x86_64-linux
    - os: ubuntu-24.04-arm, platform: aarch64-linux
    - os: macos-latest,     platform: aarch64-darwin
    - os: ubuntu-latest,    platform: aarch64-android   ← NEW
```

## Detailed changes to `.github/workflows/release.yml`

### 1. Matrix definition

Replace the current `matrix.os` with explicit `include` entries:

```yaml
build:
  needs: tag
  runs-on: ${{ matrix.os }}
  strategy:
    matrix:
      include:
        - os: ubuntu-latest
          platform: x86_64-linux
        - os: ubuntu-24.04-arm
          platform: aarch64-linux
        - os: macos-latest
          platform: aarch64-darwin
        - os: ubuntu-latest
          platform: aarch64-android
```

### 2. System dependencies step

Add Android NDK installation:

```yaml
- name: Install system dependencies (Linux)
  if: startsWith(matrix.os, 'ubuntu') && matrix.platform != 'aarch64-android'
  run: sudo apt-get update && sudo apt-get install -y libasound2-dev pandoc

- name: Install system dependencies (macOS)
  if: startsWith(matrix.os, 'macos')
  run: brew install pkg-config pandoc

- name: Install Android NDK + tools
  if: matrix.platform == 'aarch64-android'
  uses: nttld/setup-ndk@v1
  with:
    ndk-version: r27b

- name: Install Rust Android target
  if: matrix.platform == 'aarch64-android'
  run: |
    rustup target add aarch64-linux-android
    cargo install cargo-ndk

- name: Install termux-elf-cleaner
  if: matrix.platform == 'aarch64-android'
  run: pip install termux-elf-cleaner
```

### 3. Build step

Replace the single `cargo build --release` with conditional commands:

```yaml
- name: Build release binaries
  if: matrix.platform != 'aarch64-android'
  run: cargo build --release

- name: Build Android release binaries
  if: matrix.platform == 'aarch64-android'
  run: |
    CARGO_INCREMENTAL=0 cargo ndk -t arm64-v8a -p 27 \
      build --release --no-default-features --features pulseaudio

- name: Clean Android ELF sections
  if: matrix.platform == 'aarch64-android'
  run: |
    termux-elf-cleaner target/aarch64-linux-android/release/gtmd
    termux-elf-cleaner target/aarch64-linux-android/release/gtm
```

### 4. Manpages and completions

Skip for Android (Termux doesn't use systemd, manpages are optional):

```yaml
- name: Generate manpages (pandoc)
  if: matrix.platform != 'aarch64-android'
  run: ./scripts/gen-manpages.sh artifacts

- name: Generate shell completions
  if: matrix.platform != 'aarch64-android'
  run: cargo run --release --bin release-gen completions artifacts
```

### 5. Binary packaging

Android binaries are in a different path (`target/aarch64-linux-android/release/`):

```yaml
- name: Package binaries (desktop)
  if: matrix.platform != 'aarch64-android'
  shell: bash
  run: |
    cd target/release
    for bin in gtmd gtm; do
      cp "${bin}" "${bin}-${{ matrix.platform }}"
    done

- name: Package binaries (Android)
  if: matrix.platform == 'aarch64-android'
  shell: bash
  run: |
    cd target/aarch64-linux-android/release
    for bin in gtmd gtm; do
      cp "${bin}" "${bin}-aarch64-android"
    done
```

### 6. Archives

Android gets a simplified archive (no systemd service, no completions):

```yaml
- name: Create Android archive
  if: matrix.platform == 'aarch64-android'
  run: |
    cd target/aarch64-linux-android/release
    tar czf gtm-full-aarch64-android.tar.gz \
      gtm-aarch64-android \
      gtmd-aarch64-android
    tar czf gtmd-aarch64-android.tar.gz \
      gtmd-aarch64-android
```

### 7. Debian packages (Termux .deb)

```yaml
- name: Build Termux .deb packages
  if: matrix.platform == 'aarch64-android'
  run: |
    pip install termux-create-package
    make deb-termux

- name: Collect Android artifacts
  if: matrix.platform == 'aarch64-android'
  run: |
    mkdir -p android-artifacts
    cp target/aarch64-linux-android/release/gtm-full-aarch64-android.tar.gz android-artifacts/
    cp target/aarch64-linux-android/release/gtmd-aarch64-android.tar.gz android-artifacts/
    cp target/debian/*_aarch64.deb android-artifacts/ 2>/dev/null || true
```

### 8. Upload artifacts

Extend the upload step:

```yaml
- uses: actions/upload-artifact@v4
  with:
    name: artifacts-${{ matrix.platform }}
    path: |
      target/release/gtm-full-*
      target/release/gtmd-*
      target/aarch64-linux-android/release/gtm-full-*
      target/aarch64-linux-android/release/gtmd-*
      artifacts/
      target/debian/*.deb
      android-artifacts/*.deb
```

### 9. Release job

Add Android files to the release:

```yaml
- uses: softprops/action-gh-release@v2
  with:
    files: |
      # Existing
      target/release/gtm-full-*-linux
      target/release/gtm-full-*-darwin
      target/release/gtmd-*-linux
      target/release/gtmd-*-darwin
      target/release/gtm-full-*.tar.gz
      target/release/gtmd-*.tar.gz
      target/release/gtm-*.tar.gz
      artifacts/man/*
      artifacts/completions/*
      target/debian/*.deb
      # New — Android
      target/aarch64-linux-android/release/gtm-full-*
      target/aarch64-linux-android/release/gtmd-*
      android-artifacts/*.deb
```

## Summary of artifacts per platform

| Platform | Archives | .deb | Manpages | Completions |
|---|---|---|---|---|
| x86_64-linux | gtm-full, gtmd, gtm .tar.gz | Yes (cargo-deb) | Yes | Yes |
| aarch64-linux | gtm-full, gtmd, gtm .tar.gz | Yes (cargo-deb) | Yes | Yes |
| aarch64-darwin | gtm-full, gtmd, gtm .tar.gz | No | Yes | Yes |
| aarch64-android | gtm-full, gtmd .tar.gz | Yes (termux) | No | No |

## CI time estimate

The Android build adds ~5-8 minutes to the pipeline:
- NDK setup: ~30s
- Cross-compilation: ~4-6min (release mode, LTO)
- ELF cleaning: ~5s
- Packaging: ~10s

## Testing the workflow

Before merging, verify with a dry run:

```bash
# Local simulation
CARGO_INCREMENTAL=0 cargo ndk -t arm64-v8a -p 27 \
  build --release --no-default-features --features pulseaudio

termux-elf-cleaner target/aarch64-linux-android/release/gtmd
termux-elf-cleaner target/aarch64-linux-android/release/gtm

# Check binary
file target/aarch64-linux-android/release/gtmd
# → ELF 64-bit LSB executable, ARM aarch64, ... Android
```
