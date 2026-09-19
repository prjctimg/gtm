#!/usr/bin/env bash
# gtm installer — see https://github.com/prjctimg/gtm.rs
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/prjctimg/gtm.rs/main/install.sh | bash
#   install.sh                        # download and install the release for this system
#   install.sh --version 0.2.73       # pin a specific release
#   install.sh --nightly              # install the latest nightly prerelease
#   install.sh --prefix ~/.local      # install under a custom prefix
#
# When run from inside a gtm release archive (bin/gtm + bin/gtmd sit next to
# this script) the same file installs the bundled assets directly. The
# standalone form is a thin bootstrap: it downloads the per-platform archive,
# extracts it, and runs ./install.sh from inside the archive — so the installer
# logic lives in a single file that is shipped in every archive.
#
# Recognised standard environment variables (all overridable):
#   PREFIX DATAROOTDIR DATADIR BINDIR MANDIR SYSTEMD_DIR APPLICATIONS_DIR
#   ICONS_DIR XDG_DATA_HOME XDG_CONFIG_HOME ZDOTDIR BASH_COMPLETION_DIR
#   ZSH_COMPLETION_DIR FISH_COMPLETION_DIR ELVISH_COMPLETION_DIR
#   POWERSHELL_COMPLETION_DIR

set -euo pipefail

REPO="prjctimg/gtm.rs"

NC='\033[0m'
MUTED='\033[0;2m'
RED='\033[0;31m'
ORANGE='\033[38;5;214m'
GREEN='\033[0;32m'
BOLD='\033[1m'

usage() {
  cat <<EOF
gtm installer — https://github.com/${REPO}

Usage: install.sh [options]

Options:
  -h, --help            Show this help message
  -v, --version <ver>   Install a specific version (e.g. 0.2.73)
      --nightly         Install the latest nightly prerelease
  -p, --prefix <dir>    Install prefix for the tarball (default: \$HOME/.local)
  -y, --yes             Non-interactive (accepted for compatibility)

When run from inside a release archive this file installs the bundled
binaries, man pages, completions, systemd unit, desktop entry and icon.

Examples:
  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | bash
  install.sh --version 0.2.73
  install.sh --prefix /usr/local
EOF
}

# Logging helpers — all go to stderr so `install.sh | tee log` stays usable.
info() { printf "${MUTED}%s${NC}\n" "$*" >&2; }
log() { printf "${NC}%s\n" "$*" >&2; }
ok() { printf "${GREEN}%s${NC}\n" "$*" >&2; }
warn() { printf "${ORANGE}%s${NC}\n" "$*" >&2; }
die() {
  printf "${RED}%s${NC}\n" "$*" >&2
  exit 1
}
need() {
  command -v "$1" >/dev/null 2>&1 || die "requires '$1' — install it first, or download a release archive manually"
}
# Colourised header for an install step with emoji (binaries, man pages, …).
step() { printf "${BOLD}${GREEN}==>${NC} ${BOLD}%s${NC}\n" "$*" >&2; }

# Download a URL with a simple message. The progress indicator was removed
# because it was showing incorrect file size and downloaded size.
#   download_simple <url> <outfile>
download_simple() {
  local url="$1" out="$2"
  curl -fL "$url" -o "$out" 2>/dev/null
  return $?
}

VERSION=""
CHANNEL="stable"
PREFIX="${PREFIX:-$HOME/.local}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h | --help)
      usage
      exit 0
      ;;
    --nightly)
      CHANNEL="nightly"
      shift
      ;;
    -v | --version)
      if [[ -n "${2:-}" ]]; then
        VERSION="$2"
        shift 2
      else
        die "--version requires a version argument"
      fi
      ;;
    -p | --prefix)
      if [[ -n "${2:-}" ]]; then
        PREFIX="$2"
        shift 2
      else
        die "--prefix requires a directory argument"
      fi
      ;;
    -y | --yes)
      shift
      ;;
    *)
      die "unknown option: $1 (see --help)"
      ;;
  esac
done

# Detect whether this copy lives inside a release archive (bin/gtm + bin/gtmd
# sit next to it). When piped through `curl | bash` the script path cannot be
# resolved and we always take the bootstrap path.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd 2>/dev/null || true)"
IN_ARCHIVE=0
if [ -n "${SCRIPT_DIR}" ] && [ -f "${SCRIPT_DIR}/bin/gtm" ] && [ -f "${SCRIPT_DIR}/bin/gtmd" ]; then
  IN_ARCHIVE=1
fi

# ── Platform resolution (shared by both modes) ────────────────────────────────

OS=""
ARCH=""
PLATFORM=""

detect_platform() {
  OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
  if [ "$(uname -o 2>/dev/null)" = "Android" ] || [ -n "${TERMUX_VERSION:-}" ]; then
    OS="android"
  fi
  case "${OS}" in
    linux | darwin | android) ;;
    *) die "unsupported OS: ${OS} (expected linux, darwin, or android/termux)" ;;
  esac

  ARCH="$(uname -m)"
  case "${ARCH}" in
    x86_64 | amd64) ARCH="x86_64" ;;
    aarch64 | arm64) ARCH="aarch64" ;;
    *) die "unsupported architecture: ${ARCH} (expected x86_64 or aarch64)" ;;
  esac

  if [ "${OS}" = "android" ]; then
    # Termux CI publishes aarch64 builds only.
    PLATFORM="aarch64-android"
  elif [ "${OS}" = "darwin" ]; then
    PLATFORM="aarch64-darwin"
  else
    # Linux — Arch, musl (Alpine-style) or glibc (Debian) archive.
    if [ -f /etc/arch-release ] || command -v pacman >/dev/null 2>&1; then
      PLATFORM="arch-${ARCH}"
    elif [ -f /etc/alpine-release ]; then
      PLATFORM="${ARCH}-linux-musl"
    elif command -v ldd >/dev/null 2>&1 && ldd --version 2>&1 | grep -qi musl; then
      PLATFORM="${ARCH}-linux-musl"
    else
      PLATFORM="debian-12-${ARCH}"
    fi
  fi
}

resolve_latest_stable_tag() {
  local tag
  tag="$(curl -sf "https://api.github.com/repos/${REPO}/releases/latest" \
    | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p' || true)"
  [ -n "${tag}" ] || die "could not resolve the latest stable release from GitHub"
  echo "${tag}"
}

# Resolve the download URL for <archive> on the release <tag>.
#
# The public GitHub API is consulted first: it omits draft releases, so a
# nightly that is mid-build (temporarily toggled to a draft) resolves to "not
# published yet" instead of a silent 404 from releases/download/nightly/….
# When the caller passes `strict` (nightly), a failed resolution aborts so we
# never ask curl to fetch a URL that cannot exist; otherwise (stable) we fall
# back to the conventional release URL so installs keep working even when the
# API is unreachable or rate-limited.
#   resolve_asset_url <tag> <archive> [strict]
resolve_asset_url() {
  local tag="$1" archive="$2" strict="${3:-0}"
  local direct="https://github.com/${REPO}/releases/download/${tag}/${archive}"
  local names
  names="$(curl -sf "https://api.github.com/repos/${REPO}/releases/tags/${tag}" 2>/dev/null \
    | sed 's/}, *{/\n/g' \
    | grep -o '"name": *"[^"]*"' \
    | sed 's/^"name": *"//; s/"$//')" || true
  if printf '%s\n' "${names}" | grep -qxF "${archive}"; then
    printf '%s\n' "${direct}"
    return 0
  fi
  if [ "${strict}" = 1 ]; then
    return 1
  fi
  printf '%s\n' "${direct}"
  return 0
}

# ── Bootstrap mode: download this system's archive, extract, re-run ───────────

bootstrap_install() {
  need curl
  need tar

  detect_platform
  info "platform: ${OS}/${ARCH} → ${PLATFORM}"

  local tag
  if [ "${CHANNEL}" = "nightly" ]; then
    tag="nightly"
  elif [ -n "${VERSION}" ]; then
    tag="v${VERSION#v}"
  else
    VERSION="$(resolve_latest_stable_tag)"
    tag="v${VERSION}"
    info "latest stable: v${VERSION}"
  fi

  local archive_name="gtm-${PLATFORM}.tar.gz"
  local url=""
  if [ "${CHANNEL}" = "nightly" ]; then
    # Resolve strictly against the published nightly so a draft (mid-build)
    # resolves to a clear "try again" instead of a dead 404 URL.
    url="$(resolve_asset_url "${tag}" "${archive_name}" 1)" || {
      die "nightly archive '${archive_name}' is not published yet — the latest nightly build may still be running or failed. Retry in a few minutes, or install a stable release with: install.sh --version <ver>"
    }
  else
    url="$(resolve_asset_url "${tag}" "${archive_name}")"
  fi

  # Script-scope on purpose (no `local`): the EXIT trap must still read it
  # after this function returns, or `set -u` would trip on an unbound
  # variable. Uniquely named so it can never collide with a caller-exported
  # `$tmp`, and the trap uses `:?` so an empty value fails loudly instead of
  # running `rm -rf ""`.
  BOOTSTRAP_TMPDIR="$(mktemp -d)" || die "mktemp failed"
  trap 'rm -rf "${BOOTSTRAP_TMPDIR:?}"' EXIT

  log "📥 downloading ${archive_name}..."
  if ! download_simple "${url}" "${BOOTSTRAP_TMPDIR}/${archive_name}"; then
    die "download failed: ${url}"
  fi
  ok "  downloaded ${archive_name}"

  info "extracting ${archive_name}..."
  tar -xzf "${BOOTSTRAP_TMPDIR}/${archive_name}" -C "${BOOTSTRAP_TMPDIR}"

  local extracted_dir="${BOOTSTRAP_TMPDIR}/${archive_name%.tar.gz}"
  [ -d "${extracted_dir}" ] || die "archive did not extract to ${extracted_dir}"

  info "running installer from the archive..."
  (
    cd "${extracted_dir}"
    ./install.sh "$@"
  )
}

# ── Archive mode: install the assets bundled next to this script ──────────────

install_from_archive() {
  detect_platform

  local prefix="${PREFIX}"
  local datarootdir="${DATAROOTDIR:-${prefix}/share}"
  local datadir="${DATADIR:-${datarootdir}}"
  local bindir="${BINDIR:-${prefix}/bin}"
  local mandir="${MANDIR:-${datadir}/man/man1}"

  local user_install=0
  if [ "${prefix#"$HOME"}" != "${prefix}" ]; then
    user_install=1
  fi

  local systemd_dir applications_dir icons_dir
  local bash_comp_dir zsh_comp_dir fish_comp_dir elvish_comp_dir powershell_comp_dir

  if [ "${user_install}" = 1 ]; then
    XDG_DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
    XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
    systemd_dir="${SYSTEMD_DIR:-${XDG_DATA_HOME}/systemd/user}"
    applications_dir="${APPLICATIONS_DIR:-${datadir}/applications}"
    icons_dir="${ICONS_DIR:-${datadir}/icons/hicolor/scalable/apps}"
    bash_comp_dir="${BASH_COMPLETION_DIR:-${XDG_DATA_HOME}/bash-completion/completions}"
    zsh_comp_dir="${ZSH_COMPLETION_DIR:-${ZDOTDIR:-$HOME}/.zsh/completions}"
    fish_comp_dir="${FISH_COMPLETION_DIR:-${XDG_CONFIG_HOME}/fish/completions}"
    elvish_comp_dir="${ELVISH_COMPLETION_DIR:-$HOME/.elvish/lib}"
    powershell_comp_dir="${POWERSHELL_COMPLETION_DIR:-${XDG_DATA_HOME}/powershell/Modules}"
  else
    systemd_dir="${SYSTEMD_DIR:-${datarootdir}/systemd/user}"
    applications_dir="${APPLICATIONS_DIR:-${datadir}/applications}"
    icons_dir="${ICONS_DIR:-${datadir}/icons/hicolor/scalable/apps}"
    bash_comp_dir="${BASH_COMPLETION_DIR:-${datadir}/bash-completion/completions}"
    zsh_comp_dir="${ZSH_COMPLETION_DIR:-${datadir}/zsh/site-functions}"
    fish_comp_dir="${FISH_COMPLETION_DIR:-${datadir}/fish/vendor_completions.d}"
    elvish_comp_dir="${ELVISH_COMPLETION_DIR:-${datadir}/elvish/lib}"
    powershell_comp_dir="${POWERSHELL_COMPLETION_DIR:-${datadir}/powershell/Modules}"
  fi

  log "installing to ${prefix} (bin: ${bindir})"
  log ""

  # Binaries
  if [ ! -d "bin" ]; then
    die "archive is missing its bin/ directory"
  fi
  mkdir -p "${bindir}"
  step "📦 binaries"
  for name in gtm gtmd; do
    if [ -f "bin/${name}" ]; then
      install -m 0755 "bin/${name}" "${bindir}/${name}"
      ok "  ${name} -> ${bindir}/${name}"
    else
      warn "missing bin/${name} — skipping"
    fi
  done

  # Man pages
  if [ -d "man/man1" ]; then
    step "📖 man pages"
    mkdir -p "${mandir}"
    for f in man/man1/*.1; do
      [ -f "${f}" ] || continue
      install -m 0644 "${f}" "${mandir}/$(basename "${f}")"
    done
    ok "  ${mandir}/"
  fi

  # Completions — place each file into the conventional directory for its shell.
  if [ -d "completions" ]; then
    step "⌨️  shell completions"
    for f in completions/*; do
      [ -f "${f}" ] || continue
      base="$(basename "${f}")"
      case "${base}" in
        gtm.bash | gtmd.bash)
          mkdir -p "${bash_comp_dir}"
          install -m 0644 "${f}" "${bash_comp_dir}/${base%.bash}"
          ;;
        _gtm | _gtmd)
          mkdir -p "${zsh_comp_dir}"
          install -m 0644 "${f}" "${zsh_comp_dir}/${base}"
          ;;
        gtm.fish | gtmd.fish)
          mkdir -p "${fish_comp_dir}"
          install -m 0644 "${f}" "${fish_comp_dir}/${base}"
          ;;
        gtm.elv | gtmd.elv)
          mkdir -p "${elvish_comp_dir}"
          install -m 0644 "${f}" "${elvish_comp_dir}/${base}"
          ;;
        gtm.ps1 | gtmd.ps1)
          mkdir -p "${powershell_comp_dir}"
          install -m 0644 "${f}" "${powershell_comp_dir}/${base}"
          ;;
      esac
    done
    ok "completions -> ${bash_comp_dir}, ${zsh_comp_dir}, ${fish_comp_dir}, ..."
  fi

  # systemd user unit (Linux only — not macOS/Android)
  if [ "${OS}" = "linux" ] && [ -f "systemd/gtmd.service" ]; then
    step "⚙️  systemd user unit"
    mkdir -p "${systemd_dir}"
    install -m 0644 "systemd/gtmd.service" "${systemd_dir}/gtmd.service"
    ok "  ${systemd_dir}/gtmd.service"
    log "enable with: systemctl --user enable --now gtmd"
  elif [ -f "systemd/gtmd.service" ]; then
    info "skipping systemd unit (no systemd on ${OS})"
  fi

  # Desktop entry + icon
  if [ -f "desktop/gtm.desktop" ]; then
    step "🖥️  desktop entry"
    mkdir -p "${applications_dir}"
    install -m 0644 "desktop/gtm.desktop" "${applications_dir}/gtm.desktop"
    ok "  ${applications_dir}/gtm.desktop"
  fi
  if [ -f "icons/gtm.svg" ]; then
    step "🎨  icon"
    mkdir -p "${icons_dir}"
    install -m 0644 "icons/gtm.svg" "${icons_dir}/gtm.svg"
    ok "  ${icons_dir}/gtm.svg"
  fi

  ok "installation complete"
  if ! echo ":${PATH}:" | grep -q ":${bindir}:"; then
    warn "${bindir} is not in your \$PATH"
    echo "  add it to your shell profile:" >&2
    echo "" >&2
    echo "    # bash/zsh: ~/.bashrc, ~/.zshrc, or ~/.profile" >&2
    echo "    export PATH=\"${bindir}:\$PATH\"" >&2
    echo "" >&2
    echo "    # fish: ~/.config/fish/config.fish" >&2
    echo "    fish_add_path ${bindir}" >&2
    echo "" >&2
  fi
}

# ── Entry point ────────────────────────────────────────────────────────────────

if [ "${IN_ARCHIVE}" = 1 ]; then
  install_from_archive
else
  bootstrap_install "$@"
fi