#!/usr/bin/env bash
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/prjctimg/gtm.rs/main/install.sh | bash
#
# Options:
#   --stable   Install latest stable release (default)
#   --nightly  Install nightly pre-release
#   --version  Pin to specific tag (e.g. v0.2.5 or 0.2.5)
#   --prefix   Prefix for tarball installs (default: $HOME/.local, ignored for .deb)
#   --yes      Non-interactive (passed to apt for .deb)

set -euo pipefail

REPO="prjctimg/gtm.rs"
VERSION="${VERSION:-}"
PREFIX="${PREFIX:-$HOME/.local}"
CHANNEL="stable"
ASSUME_YES=0

while [[ $# -gt 0 ]]; do
  case "$1" in
  --stable)
    CHANNEL="stable"
    shift
    ;;
  --nightly)
    CHANNEL="nightly"
    shift
    ;;
  --version)
    VERSION="$2"
    shift 2
    ;;
  --prefix)
    PREFIX="$2"
    shift 2
    ;;
  --yes | -y)
    ASSUME_YES=1
    shift
    ;;
  --help | -h)
    sed -n '2,22p' "$0" | sed 's/^# //;s/^#//'
    exit 0
    ;;
  *)
    echo "Unknown option: $1" >&2
    exit 1
    ;;
  esac
done

# Handle VERSION containing channel hint
if [[ "$VERSION" == "nightly" ]]; then
  CHANNEL="nightly"
  VERSION=""
fi

# ── Helpers (all to stderr to avoid Bad file descriptor when piped) ──
info() { printf '\033[1;34m▸\033[0m %s\n' "$*" >&2; }
ok() { printf '\033[1;32m✔\033[0m %s\n' "$*" >&2; }
warn() { printf '\033[1;33m⚠\033[0m %s\n' "$*" >&2; }
die() {
  printf '\033[1;31m✖\033[0m %s\n' "$*" >&2
  exit 1
}

command -v curl >/dev/null 2>&1 || die "curl is required but not found"
command -v tar >/dev/null 2>&1 || die "tar is required but not found"

OS=$(uname -s | tr '[:upper:]' '[:lower:]')
if [ "$(uname -o 2>/dev/null)" = "Android" ] || [ -n "${TERMUX_VERSION:-}" ]; then
  OS="android"
fi

case "$OS" in
linux | darwin | android) ;;
*) die "Unsupported OS: $OS (expected Linux, macOS, or Android)" ;;
esac

ARCH=$(uname -m)
case "$ARCH" in
x86_64 | amd64) ARCH="x86_64" ;;
aarch64 | arm64) ARCH="aarch64" ;;
*) die "Unsupported architecture: $ARCH (expected x86_64 or aarch64)" ;;
esac

DISTRO=""
DEB_VER=""
if [ "$OS" = "linux" ] && [ -f /etc/os-release ]; then
  # shellcheck disable=SC1091
  . /etc/os-release 2>/dev/null || true
  DISTRO="${ID:-}"
  DEB_VER="${VERSION_ID:-}"
fi

PLATFORM="${ARCH}-${OS}"
if [ "$DISTRO" = "alpine" ]; then
  PLATFORM="${ARCH}-linux-musl"
fi

info "Platform: ${PLATFORM} (OS=$OS ARCH=$ARCH ID=${DISTRO:-unknown})"
info "Channel: ${CHANNEL} ${VERSION:+VERSION=$VERSION}"

resolve_latest_stable_tag() {
  local tag
  tag=$(curl -sf "https://api.github.com/repos/${REPO}/releases?per_page=20" |
    grep -o '"tag_name":"v[^"]*"' |
    grep -v -e '-rc' -e '-beta' -e '-alpha' -e '-dev' |
    grep -v '"tag_name":"nightly"' |
    head -1 |
    sed 's/"tag_name":"v//;s/"//' || true)
  if [ -z "$tag" ]; then
    tag=$(curl -sf "https://api.github.com/repos/${REPO}/releases?per_page=20" |
      grep -o '"tag_name":"v[^"]*"' |
      head -1 |
      sed 's/"tag_name":"v//;s/"//' || true)
  fi
  [ -n "$tag" ] || die "Could not resolve latest stable tag from GitHub"
  echo "$tag"
}

TAG=""
RESOLVED_VERSION=""
if [ "$CHANNEL" = "nightly" ]; then
  TAG="nightly"
  RESOLVED_VERSION="nightly"
else
  if [ -n "$VERSION" ]; then
    RESOLVED_VERSION="${VERSION#v}"
    TAG="v${RESOLVED_VERSION}"
  else
    probe_ver=""
    for bin in gtmd gtm; do
      if command -v "$bin" >/dev/null 2>&1; then
        # timeout 2s if available
        if command -v timeout >/dev/null 2>&1; then
          probe_ver=$(timeout 2 "$bin" --version 2>/dev/null | head -n1 | awk '{print $2}' || true)
        else
          probe_ver=$("$bin" --version 2>/dev/null | head -n1 | awk '{print $2}' || true)
        fi
        if [[ "$probe_ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then
          probe_ver="${probe_ver#v}"
          break
        fi
        probe_ver=""
      fi
    done
    if [ -n "$probe_ver" ]; then
      info "Detected installed version: ${probe_ver} — upgrading to latest stable"
    fi
    RESOLVED_VERSION=$(resolve_latest_stable_tag)
    TAG="v${RESOLVED_VERSION}"
    info "Latest stable: ${TAG}"
  fi
fi

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

USE_DEB=0
DEB_ARCH=""
if [ "$OS" = "linux" ] && command -v dpkg >/dev/null 2>&1; then
  # Only use deb on Debian/Ubuntu family; not on Alpine/musl
  if [ "$DISTRO" = "debian" ] || [ "$DISTRO" = "ubuntu" ] || [ -f /etc/debian_version ]; then
    # Ensure platform matches deb arch: x86_64 → amd64, aarch64 → arm64
    DEB_ARCH=$(dpkg --print-architecture 2>/dev/null || true)
    case "$DEB_ARCH" in
    amd64 | arm64) USE_DEB=1 ;;
    *)
      warn "dpkg arch $DEB_ARCH not amd64/arm64 — falling back to tarball"
      USE_DEB=0
      ;;
    esac
    # Cross-check: if PLATFORM arch and DEB_ARCH mismatch, trust DEB_ARCH but warn
    if [ "$ARCH" = "x86_64" ] && [ "$DEB_ARCH" != "amd64" ]; then
      warn "ARCH $ARCH vs dpkg $DEB_ARCH mismatch — using deb arch $DEB_ARCH"
    fi
    if [ "$ARCH" = "aarch64" ] && [ "$DEB_ARCH" != "arm64" ]; then
      warn "ARCH $ARCH vs dpkg $DEB_ARCH mismatch — using deb arch $DEB_ARCH"
    fi
    if [ "$CHANNEL" = "nightly" ] && [ -z "$VERSION" ]; then
      if [ -z "$RESOLVED_VERSION" ] || [ "$RESOLVED_VERSION" = "nightly" ]; then
        RESOLVED_VERSION=$(resolve_latest_stable_tag)
      fi
    fi
  fi
fi

if [ "$USE_DEB" = 1 ]; then
  if [ "$PREFIX" != "$HOME/.local" ]; then
    warn "--prefix is ignored for .deb installs (system package)"
  fi
  DEB_VERSION="$RESOLVED_VERSION"
  if [ "$DEB_VERSION" = "nightly" ]; then
    DEB_VERSION=$(curl -sf "https://raw.githubusercontent.com/${REPO}/main/Cargo.toml" | grep -m1 '^version = ' | sed 's/^version = "\(.*\)"/\1/' || true)
    [ -n "$DEB_VERSION" ] || DEB_VERSION=$(resolve_latest_stable_tag)
  fi
  if [ "$OS" = "android" ]; then
    DEB_FILE="gtm_${DEB_VERSION}_aarch64.deb"
  else
    DEB_FILE="gtm_${DEB_VERSION}_${DEB_ARCH}.deb"
  fi

  DEB_URL="https://github.com/${REPO}/releases/download/${TAG}/${DEB_FILE}"
  DEB_PATH="${TMPDIR}/${DEB_FILE}"

  info "Downloading .deb: ${DEB_URL}"
  if ! curl -#fL "$DEB_URL" -o "$DEB_PATH"; then
    warn "Failed to download .deb, falling back to tarball"
    USE_DEB=0
  else
    info "Installing ${DEB_FILE}..."
    if [ "$OS" = "android" ]; then
      dpkg -i "$DEB_PATH" || {
        info "Fixing dependencies..."
        pkg install -y -f 2>/dev/null || true
        dpkg -i "$DEB_PATH"
      }
    else
      if [ "$(id -u)" != "0" ]; then
        command -v sudo >/dev/null 2>&1 || die "sudo is required to install the .deb package"
        sudo DEBIAN_FRONTEND=noninteractive apt-get update -qq 2>&1 | tail -n 20 >&2 || true
        sudo DEBIAN_FRONTEND=noninteractive apt-get install -y "$DEB_PATH"
      else
        DEBIAN_FRONTEND=noninteractive apt-get update -qq 2>&1 | tail -n 20 >&2 || true
        DEBIAN_FRONTEND=noninteractive apt-get install -y "$DEB_PATH"
      fi
    fi
    ok "Installation complete (deb)!"
    exit 0
  fi
fi

ASSET_PLATFORM="$PLATFORM"
if [ "$OS" = "linux" ] && [ "$DISTRO" != "alpine" ]; then
  ASSET_PLATFORM="debian-12-${ARCH}"
fi

TARBALL_URL="https://github.com/${REPO}/releases/download/${TAG}/gtm-${ASSET_PLATFORM}.tar.gz"
TARBALL="${TMPDIR}/gtm-${ASSET_PLATFORM}.tar.gz"

info "Downloading: ${TARBALL_URL}"
if ! curl -#fL "$TARBALL_URL" -o "$TARBALL"; then
  die "Failed to download ${TARBALL_URL} (platform ${ASSET_PLATFORM} may not have a release for ${TAG})"
fi

info "Extracting..."
tar xzf "$TARBALL" -C "$TMPDIR"

SRC="${TMPDIR}/gtm-${ASSET_PLATFORM}"
if [ ! -d "$SRC" ]; then
  SRC="$TMPDIR"
fi
BIN_DIR="${SRC}/bin"
MAN_DIR="${SRC}/man/man1"
COMPLETIONS_DIR="${SRC}/completions"
SYSTEMD_FILE="${SRC}/systemd/gtmd.service"
DESKTOP_FILE="${SRC}/desktop/gtm.desktop"
ICON_FILE="${SRC}/icons/gtm.svg"

DATAROOTDIR="${DATAROOTDIR:-${PREFIX}/share}"
DATADIR="${DATADIR:-${DATAROOTDIR}}"
BINDIR="${BINDIR:-${PREFIX}/bin}"
MANDIR="${MANDIR:-${DATADIR}/man/man1}"
SYSTEMD_DIR="${SYSTEMD_DIR:-${PREFIX}/lib/systemd/user}"
APPLICATIONS_DIR="${APPLICATIONS_DIR:-${DATADIR}/applications}"
ICONS_DIR="${ICONS_DIR:-${DATADIR}/icons/hicolor/scalable/apps}"
if [ "${PREFIX#"$HOME"}" != "$PREFIX" ]; then
  XDG_DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
  XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
  BASH_COMPLETION_DIR="${BASH_COMPLETION_DIR:-$XDG_DATA_HOME/bash-completion/completions}"
  ZSH_COMPLETION_DIR="${ZSH_COMPLETION_DIR:-${ZDOTDIR:-$HOME}/.zsh/completions}"
  FISH_COMPLETION_DIR="${FISH_COMPLETION_DIR:-$XDG_CONFIG_HOME/fish/completions}"
  ELVISH_COMPLETION_DIR="${ELVISH_COMPLETION_DIR:-$HOME/.elvish/lib}"
  POWERSHELL_COMPLETION_DIR="${POWERSHELL_COMPLETION_DIR:-$XDG_DATA_HOME/powershell/Modules}"
else
  BASH_COMPLETION_DIR="${BASH_COMPLETION_DIR:-${DATADIR}/bash-completion/completions}"
  ZSH_COMPLETION_DIR="${ZSH_COMPLETION_DIR:-${DATADIR}/zsh/site-functions}"
  FISH_COMPLETION_DIR="${FISH_COMPLETION_DIR:-${DATADIR}/fish/vendor_completions.d}"
  ELVISH_COMPLETION_DIR="${ELVISH_COMPLETION_DIR:-${DATADIR}/elvish/lib}"
  POWERSHELL_COMPLETION_DIR="${POWERSHELL_COMPLETION_DIR:-${DATADIR}/powershell/Modules}"
fi

install -d "$BINDIR" "$MANDIR" 2>/dev/null || mkdir -p "$BINDIR" "$MANDIR"

install_bin() {
  local src="$1" name="$2"
  if [ ! -f "$src" ]; then
    warn "Missing $src — skipping $name"
    return 0
  fi
  install -Dm 0755 "$src" "${BINDIR}/${name}"
  ok "${name} -> ${BINDIR}/${name}"
}

if [ -d "$BIN_DIR" ]; then
  info "Installing binaries to ${BINDIR}..."
  install_bin "${BIN_DIR}/gtm" "gtm"
  install_bin "${BIN_DIR}/gtmd" "gtmd"
else
  die "Archive missing bin/ directory"
fi

# Man pages
if [ -d "$MAN_DIR" ]; then
  for f in "$MAN_DIR"/*.1; do
    [ -f "$f" ] || continue
    install -Dm 0644 "$f" "${MANDIR}/$(basename "$f")"
  done
  ok "Man pages -> ${MANDIR}/"
fi

# Enable completions for the user's login shell (best-effort, idempotent).
enable_login_shell_completion() {
  [ -d "$COMPLETIONS_DIR" ] || return 0
  local shell_name
  shell_name=$(basename "${SHELL:-/bin/bash}")
  case "$shell_name" in
    zsh)
      local rc="${ZDOTDIR:-$HOME}/.zshrc"
      if [ -d "$ZSH_COMPLETION_DIR" ] && ! grep -q "gtm completion (zsh)" "$rc" 2>/dev/null; then
        {
          echo ""
          echo "# gtm completion (zsh)"
          echo "fpath=(${ZSH_COMPLETION_DIR} \$fpath)"
          echo "autoload -Uz compinit && compinit"
        } >> "$rc"
        ok "Enabled zsh completions in ${rc} (restart shell or run 'exec zsh')"
      fi
      ;;
    fish)
      [ -d "$FISH_COMPLETION_DIR" ] && ok "fish completions auto-load from ${FISH_COMPLETION_DIR}"
      ;;
    elvish)
      if [ -d "$ELVISH_COMPLETION_DIR" ]; then
        info "elvish: enable completions by adding to ~/.elvish/rc.elv:"
        echo "  set paths = [\$@paths ${ELVISH_COMPLETION_DIR}]" >&2
        echo "  use gtm" >&2
        echo "  use gtmd" >&2
      fi
      ;;
    powershell | pwsh)
      if [ -d "$POWERSHELL_COMPLETION_DIR" ]; then
        info "PowerShell: enable completions by adding to your \$PROFILE:"
        echo "  \$env:PSModulePath += [IO.Path]::PathSeparator + '${POWERSHELL_COMPLETION_DIR}'" >&2
        echo "  Import-Module gtm" >&2
        echo "  Import-Module gtmd" >&2
      fi
      ;;
    *)
      [ -d "$BASH_COMPLETION_DIR" ] && ok "bash completions auto-load from ${BASH_COMPLETION_DIR}"
      ;;
  esac
}

# Completions (handle both legacy naming and new)
if [ -d "$COMPLETIONS_DIR" ]; then
  for f in "$COMPLETIONS_DIR"/*; do
    [ -f "$f" ] || continue
    base=$(basename "$f")
    case "$base" in
    gtm.bash) install -Dm 0644 "$f" "${BASH_COMPLETION_DIR}/gtm" ;;
    gtmd.bash) install -Dm 0644 "$f" "${BASH_COMPLETION_DIR}/gtmd" ;;
    _gtm) install -Dm 0644 "$f" "${ZSH_COMPLETION_DIR}/_gtm" ;;
    _gtmd) install -Dm 0644 "$f" "${ZSH_COMPLETION_DIR}/_gtmd" ;;
    gtm.fish) install -Dm 0644 "$f" "${FISH_COMPLETION_DIR}/gtm.fish" ;;
    gtmd.fish) install -Dm 0644 "$f" "${FISH_COMPLETION_DIR}/gtmd.fish" ;;
    gtm.elv) install -Dm 0644 "$f" "${ELVISH_COMPLETION_DIR}/gtm.elv" ;;
    gtmd.elv) install -Dm 0644 "$f" "${ELVISH_COMPLETION_DIR}/gtmd.elv" ;;
    gtm.ps1) install -Dm 0644 "$f" "${POWERSHELL_COMPLETION_DIR}/gtm.ps1" ;;
    gtmd.ps1) install -Dm 0644 "$f" "${POWERSHELL_COMPLETION_DIR}/gtmd.ps1" ;;
    *)
      mkdir -p "$BASH_COMPLETION_DIR"
      cp "$f" "$BASH_COMPLETION_DIR/" 2>/dev/null || true
      ;;
    esac
  done
  ok "Completions installed"
fi

# Enable completions for the detected login shell (idempotent).
enable_login_shell_completion

# Systemd service
if [ -f "$SYSTEMD_FILE" ]; then
  install -Dm 0644 "$SYSTEMD_FILE" "${SYSTEMD_DIR}/gtmd.service"
  ok "Systemd service -> ${SYSTEMD_DIR}/gtmd.service"
fi

# Desktop entry & icon
if [ -f "$DESKTOP_FILE" ]; then
  install -Dm 0644 "$DESKTOP_FILE" "${APPLICATIONS_DIR}/gtm.desktop"
  ok "Desktop entry -> ${APPLICATIONS_DIR}/gtm.desktop"
fi
if [ -f "$ICON_FILE" ]; then
  install -Dm 0644 "$ICON_FILE" "${ICONS_DIR}/gtm.svg"
  ok "Icon -> ${ICONS_DIR}/gtm.svg"
fi

echo "" >&2
ok "Installation complete 🎊 "

if ! echo ":$PATH:" | grep -q ":${BINDIR}:"; then
  warn "${BINDIR} is not in your \$PATH."
  echo "  Add this to your shell profile:" >&2
  echo "" >&2
  echo "    export PATH=\"${BINDIR}:\$PATH\"" >&2
  echo "" >&2
fi
