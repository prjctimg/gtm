#!/usr/bin/env bash
# GTM installer — pipe-to-shell
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/prjctimg/gtm.rs/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/prjctimg/gtm.rs/main/install.sh | bash -s -- --type full
#   curl -fsSL https://raw.githubusercontent.com/prjctimg/gtm.rs/main/install.sh | bash -s -- --type deb
#
# Environment variables:
#   VERSION        – release tag to install (default: auto-detect from installed gtmd, or latest)
#   PREFIX         – install prefix (default: $HOME/.local)
#   INSTALL_TYPE   – minimal | full | tui-only | deb (skips interactive prompt)
#
set -euo pipefail

REPO="prjctimg/gtm.rs"
VERSION="${VERSION:-}"
PREFIX="${PREFIX:-$HOME/.local}"
INSTALL_TYPE="${INSTALL_TYPE:-}"

# ── Parse --type flag ───────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    --type) INSTALL_TYPE="$2"; shift 2 ;;
    --deb) INSTALL_TYPE="deb"; shift ;;
    --version) VERSION="$2"; shift 2 ;;
    --prefix) PREFIX="$2"; shift 2 ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

# ── Helpers ─────────────────────────────────────────────────────────
info()  { printf '\033[1;34m▸\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m✔\033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33m⚠\033[0m %s\n' "$*"; }
die()   { printf '\033[1;31m✖\033[0m %s\n' "$*" >&2; exit 1; }

command -v curl >/dev/null 2>&1 || die "curl is required but not found"
command -v tar  >/dev/null 2>&1 || die "tar is required but not found"

# ── Platform detection ──────────────────────────────────────────────
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
if [ "$(uname -o 2>/dev/null)" = "Android" ] || [ -n "${TERMUX_VERSION:-}" ]; then
  OS="android"
fi

case "$OS" in
  linux|darwin|android) ;;
  *) die "Unsupported OS: $OS (expected Linux, macOS, or Android)" ;;
esac

ARCH=$(uname -m)
case "$ARCH" in
  x86_64|amd64)  ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) die "Unsupported architecture: $ARCH (expected x86_64 or aarch64)" ;;
esac

PLATFORM="${ARCH}-${OS}"
info "Platform: ${PLATFORM}"

# ── Version resolution ──────────────────────────────────────────────
resolve_version() {
  if [ -n "$VERSION" ]; then
    VERSION="${VERSION#v}"
    echo "$VERSION"
    return
  fi

  # Try to detect from installed gtmd
  if command -v gtmd >/dev/null 2>&1; then
    local raw ver
    raw=$(gtmd --version 2>/dev/null | head -n1 || true)
    ver=$(printf '%s\n' "$raw" | awk '{print $2}')
    if [[ "$ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then
      info "Detected installed gtmd version: ${ver}"
      echo "$ver"
      return
    fi
  fi

  # Fall back to latest release
  info "No VERSION set and gtmd not found — using latest release"
  echo "latest"
}

VERSION=$(resolve_version)

# ── Temp directory ──────────────────────────────────────────────────
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

# ── GitHub release helpers ──────────────────────────────────────────
# GitHub returns 404 for /releases/latest/download/ if the latest release is a
# pre-release (e.g. the `nightly` build), so we fall back to the API to find the
# newest non-prerelease tag.
resolve_latest_tag() {
  local tag
  tag=$(curl -sf "https://api.github.com/repos/${REPO}/releases?per_page=20" \
    | grep -o '"tag_name":"v[^"]*"' \
    | grep -v -e '-rc' -e '-beta' -e '-alpha' -e '-dev' \
    | head -1 \
    | sed 's/"tag_name":"v//;s/"//' || true)

  # If every release is a prerelease, fall back to the newest tag of all.
  if [ -z "$tag" ]; then
    tag=$(curl -sf "https://api.github.com/repos/${REPO}/releases?per_page=20" \
      | grep -o '"tag_name":"v[^"]*"' \
      | head -1 \
      | sed 's/"tag_name":"v//;s/"//' || true)
  fi

  [ -n "$tag" ] || die "Could not resolve latest release tag from GitHub"
  echo "$tag"
}

resolve_download_url() {
  local archive="$1"  # e.g. gtm-full or gtmd or gtm
  local url

  if [ "$VERSION" = "latest" ]; then
    # Try /releases/latest/download/ first (fast, follows redirect)
    url="https://github.com/${REPO}/releases/latest/download/${archive}-${PLATFORM}.tar.gz"
    local http_code
    http_code=$(curl -sI -o /dev/null -w '%{http_code}' "$url" 2>/dev/null || true)
    if [ "$http_code" = "302" ] || [ "$http_code" = "301" ]; then
      echo "$url"
      return
    fi

    # Latest is a pre-release — query the API for the newest stable tag
    info "Latest GitHub release is a pre-release — querying API..."
    VERSION=$(resolve_latest_tag)
    info "Latest stable release: v${VERSION}"
  fi

  echo "https://github.com/${REPO}/releases/download/v${VERSION}/${archive}-${PLATFORM}.tar.gz"
}

resolve_deb_asset() {
  local tag="$1" prefix="$2" deb_arch="$3" n assets
  assets=$(curl -sf "https://api.github.com/repos/${REPO}/releases/tags/v${tag}" \
    | grep -o '"name":"[^"]*\.deb"' \
    | sed 's/"name":"//;s/"$//' || true)
  for n in $assets; do
    case "$n" in
      "${prefix}"*"${deb_arch}".deb) echo "$n"; return ;;
    esac
  done
}

# ── Installation type ───────────────────────────────────────────────
if [ -z "$INSTALL_TYPE" ]; then
  echo ""
  echo "  GTM Music Player Installer"
  echo "  ─────────────────────────────────────"
  echo ""
  echo "  Select installation type:"
  echo ""
  echo "    1) Minimal      – gtmd only (daemon, man pages, completions)"
  echo "    2) Full         – gtm + gtmd (TUI + daemon, man pages, completions)"
  echo "    3) TUI only     – gtm only (for existing gtmd installations)"
  echo "    4) .deb package – dpkg install (Debian/Ubuntu or Termux)"
  echo ""

  if command -v gtmd >/dev/null 2>&1; then
    echo "  ℹ  gtmd detected on PATH — option 3 will match its version."
    echo ""
  fi

  read -r -p "  Choice [1/2/3/4]: " choice
  case "$choice" in
    1) INSTALL_TYPE="minimal" ;;
    2) INSTALL_TYPE="full" ;;
    3) INSTALL_TYPE="tui-only" ;;
    4) INSTALL_TYPE="deb" ;;
    *) die "Invalid choice: $choice (expected 1, 2, 3, or 4)" ;;
  esac
fi

case "$INSTALL_TYPE" in
  minimal|full|tui-only|deb) ;;
  *) die "Invalid install type: $INSTALL_TYPE (expected minimal, full, tui-only, or deb)" ;;
esac

info "Install type: ${INSTALL_TYPE}"

# ── Map type to archive name ────────────────────────────────────────
case "$INSTALL_TYPE" in
  minimal)  ARCHIVE_NAME="gtmd" ;;
  full)     ARCHIVE_NAME="gtm-full" ;;
  tui-only) ARCHIVE_NAME="gtm" ;;
  deb)      ARCHIVE_NAME="" ;;
esac

# ── Validate tui-only requires gtmd ─────────────────────────────────
if [ "$INSTALL_TYPE" = "tui-only" ] && [ "$VERSION" = "latest" ]; then
  if ! command -v gtmd >/dev/null 2>&1; then
    die "TUI-only mode requires gtmd to be installed (for version detection).\n     Install gtmd first, or set VERSION explicitly."
  fi
fi

# ── .deb installation ───────────────────────────────────────────────
if [ "$INSTALL_TYPE" = "deb" ]; then
  command -v dpkg >/dev/null 2>&1 || die "dpkg not found — .deb install requires Debian/Ubuntu or Termux"

  if [ "$OS" = "android" ]; then
    deb_arch="aarch64"
    deb_prefix="gtm_"
  else
    deb_arch=$(dpkg --print-architecture 2>/dev/null || true)
    case "$deb_arch" in
      amd64|arm64) ;;
      *) die "Unsupported deb architecture: $deb_arch (expected amd64 or arm64)" ;;
    esac
    deb_prefix="gtm-full_"
  fi

  local_tag="$VERSION"
  if [ "$local_tag" = "latest" ]; then
    local_tag=$(resolve_latest_tag)
    info "Latest stable release: v${local_tag}"
  fi

  asset=$(resolve_deb_asset "$local_tag" "$deb_prefix" "$deb_arch")
  [ -n "$asset" ] || die "No matching .deb asset found for ${deb_prefix}*${deb_arch}.deb on v${local_tag}"

  DEB="${TMPDIR}/${asset}"
  info "Downloading: https://github.com/${REPO}/releases/download/v${local_tag}/${asset}"
  curl -#fL "https://github.com/${REPO}/releases/download/v${local_tag}/${asset}" -o "$DEB"

  info "Installing ${asset}..."
  if [ "$OS" = "android" ]; then
    dpkg -i "$DEB" || {
      info "Fixing dependencies..."
      pkg install -y -f 2>/dev/null || true
      dpkg -i "$DEB"
    }
  else
    if [ "$(id -u)" != "0" ]; then
      command -v sudo >/dev/null 2>&1 || die "sudo is required to install the .deb package"
      sudo dpkg -i "$DEB"
      sudo apt-get -f install -y >/dev/null 2>&1 || true
    else
      dpkg -i "$DEB"
      apt-get -f install -y >/dev/null 2>&1 || true
    fi
  fi

  echo ""
  ok "Installation complete!"
  exit 0
fi

# ── Download (tarball) ──────────────────────────────────────────────
DOWNLOAD_URL=$(resolve_download_url "$ARCHIVE_NAME")
TARBALL="${TMPDIR}/${ARCHIVE_NAME}-${PLATFORM}.tar.gz"

info "Downloading: ${DOWNLOAD_URL}"
curl -#fL "$DOWNLOAD_URL" -o "$TARBALL"

# ── Extract ─────────────────────────────────────────────────────────
info "Extracting..."
tar xzf "$TARBALL" -C "$TMPDIR"

# ── Install ─────────────────────────────────────────────────────────
BINDIR="${PREFIX}/bin"
MANDIR="${PREFIX}/share/man/man1"
BASH_COMPLETION_DIR="${PREFIX}/share/bash-completion/completions"
ZSH_COMPLETION_DIR="${PREFIX}/share/zsh/vendor-completions"
FISH_COMPLETION_DIR="${PREFIX}/share/fish/vendor_completions.d"
SYSTEMD_DIR="${PREFIX}/lib/systemd/user"

install_bin() {
  local src="$1" name="$2"
  install -Dm 0755 "$src" "${BINDIR}/${name}"
  ok "${name} -> ${BINDIR}/${name}"
}

install_man() {
  local src="$1"
  install -Dm 0644 "$src" "${MANDIR}/$(basename "$src")"
}

install_completion() {
  local src="$1" dest_dir="$2"
  mkdir -p "$dest_dir"
  cp "$src" "$dest_dir/"
}

case "$INSTALL_TYPE" in
  minimal)
    info "Installing gtmd (minimal)..."
    install_bin "${TMPDIR}/gtmd-${PLATFORM}" "gtmd"

    # Man pages
    for f in "${TMPDIR}"/gtmd.1 "${TMPDIR}"/gtmd-ipc.1; do
      [ -f "$f" ] && install_man "$f"
    done
    ok "Man pages -> ${MANDIR}/"

    # Completions
    [ -f "${TMPDIR}/gtmd.bash" ] && install_completion "${TMPDIR}/gtmd.bash" "$BASH_COMPLETION_DIR"
    [ -f "${TMPDIR}/_gtmd" ]     && install_completion "${TMPDIR}/_gtmd"     "$ZSH_COMPLETION_DIR"
    [ -f "${TMPDIR}/gtmd.fish" ] && install_completion "${TMPDIR}/gtmd.fish" "$FISH_COMPLETION_DIR"
    ok "Completions installed"

    # Systemd service
    if [ -f "${TMPDIR}/gtmd.service" ]; then
      install -Dm 0644 "${TMPDIR}/gtmd.service" "${SYSTEMD_DIR}/gtmd.service"
      ok "Systemd service -> ${SYSTEMD_DIR}/gtmd.service"
    fi
    ;;

  full)
    info "Installing gtm + gtmd (full)..."
    install_bin "${TMPDIR}/gtm-${PLATFORM}"  "gtm"
    install_bin "${TMPDIR}/gtmd-${PLATFORM}" "gtmd"

    # Man pages (all)
    for f in "${TMPDIR}"/gtm.1 "${TMPDIR}"/gtmd.1 "${TMPDIR}"/gtmd-ipc.1; do
      [ -f "$f" ] && install_man "$f"
    done
    ok "Man pages -> ${MANDIR}/"

    # Completions — gtm
    [ -f "${TMPDIR}/gtm.bash" ]   && install_completion "${TMPDIR}/gtm.bash"   "$BASH_COMPLETION_DIR"
    [ -f "${TMPDIR}/_gtm" ]       && install_completion "${TMPDIR}/_gtm"       "$ZSH_COMPLETION_DIR"
    [ -f "${TMPDIR}/gtm.fish" ]   && install_completion "${TMPDIR}/gtm.fish"   "$FISH_COMPLETION_DIR"
    # Completions — gtmd
    [ -f "${TMPDIR}/gtmd.bash" ]  && install_completion "${TMPDIR}/gtmd.bash"  "$BASH_COMPLETION_DIR"
    [ -f "${TMPDIR}/_gtmd" ]      && install_completion "${TMPDIR}/_gtmd"      "$ZSH_COMPLETION_DIR"
    [ -f "${TMPDIR}/gtmd.fish" ]  && install_completion "${TMPDIR}/gtmd.fish"  "$FISH_COMPLETION_DIR"
    ok "Completions installed"

    # Systemd service
    if [ -f "${TMPDIR}/gtmd.service" ]; then
      install -Dm 0644 "${TMPDIR}/gtmd.service" "${SYSTEMD_DIR}/gtmd.service"
      ok "Systemd service -> ${SYSTEMD_DIR}/gtmd.service"
    fi
    ;;

  tui-only)
    info "Installing gtm (TUI only)..."
    install_bin "${TMPDIR}/gtm-${PLATFORM}" "gtm"

    # Man page
    [ -f "${TMPDIR}/gtm.1" ] && install_man "${TMPDIR}/gtm.1"
    ok "Man page -> ${MANDIR}/gtm.1"

    # Completions
    [ -f "${TMPDIR}/gtm.bash" ] && install_completion "${TMPDIR}/gtm.bash" "$BASH_COMPLETION_DIR"
    [ -f "${TMPDIR}/_gtm" ]     && install_completion "${TMPDIR}/_gtm"     "$ZSH_COMPLETION_DIR"
    [ -f "${TMPDIR}/gtm.fish" ] && install_completion "${TMPDIR}/gtm.fish" "$FISH_COMPLETION_DIR"
    ok "Completions installed"
    ;;
esac

# ── Done ────────────────────────────────────────────────────────────
echo ""
ok "Installation complete!"

if ! echo ":$PATH:" | grep -q ":${BINDIR}:"; then
  warn "${BINDIR} is not in your \$PATH."
  echo "  Add this to your shell profile:"
  echo ""
  echo "    export PATH=\"${BINDIR}:\$PATH\""
  echo ""
fi
