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
  -y, --yes             Non-interactive: never prompt (e.g. to enable gtmd)

When run from inside a release archive this file installs the bundled
binaries, man pages, completions, systemd unit, desktop entry and icon.

Examples:
  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | bash
  install.sh --version 0.2.73
  install.sh --prefix /usr/local
EOF
}

# Logging helpers — all go to stderr so `install.sh | tee log` stays usable.
# Each stage prints its heading (emoji kept, no `==>`) followed by a single
# colour-coded ✔ / ✘ marker for the outcome, like the download line below.
info() { printf "${MUTED}%s${NC}\n" "$*" >&2; }
log() { printf "${NC}%s\n" "$*" >&2; }
ok() { printf "${GREEN}✔${NC} %s\n" "$*" >&2; }
fail() { printf "${RED}✘${NC} %s\n" "$*" >&2; }
die() {
  printf "${RED}%s${NC}\n" "$*" >&2
  exit 1
}
need() {
  command -v "$1" >/dev/null 2>&1 || die "requires '$1' — install it first, or download a release archive manually"
}

# Stage heading (no `==>`, no trailing newline) + outcome markers.
stage() { printf "${BOLD}%s${NC}" "$*" >&2; }
stage_ok() { printf " ${GREEN}✔${NC}\n" >&2; }
stage_fail() { printf " ${RED}✘${NC}\n" >&2; }

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
ASSUME_YES=0

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
      ASSUME_YES=1
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

  log "📥 downloading ${archive_name}"
  if ! download_simple "${url}" "${BOOTSTRAP_TMPDIR}/${archive_name}"; then
    die "download failed: ${url}"
  fi
  ok "downloaded ${archive_name}"

  tar -xzf "${BOOTSTRAP_TMPDIR}/${archive_name}" -C "${BOOTSTRAP_TMPDIR}"

  local extracted_dir="${BOOTSTRAP_TMPDIR}/${archive_name%.tar.gz}"
  [ -d "${extracted_dir}" ] || die "archive did not extract correctly"

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

  # ── Binaries ────────────────────────────────────────────────────────────────
  if [ ! -d "bin" ] || [ ! -f "bin/gtm" ] || [ ! -f "bin/gtmd" ]; then
    die "archive is missing its bin/ assets"
  fi
  stage "📦 binaries"
  if mkdir -p "${bindir}" \
    && install -m 0755 "bin/gtm" "${bindir}/gtm" \
    && install -m 0755 "bin/gtmd" "${bindir}/gtmd"; then
    stage_ok
  else
    stage_fail
    die "could not install binaries"
  fi

  # ── Man pages ───────────────────────────────────────────────────────────────
  if [ -d "man/man1" ]; then
    stage "📖 man pages"
    if install_man_pages; then
      stage_ok
    else
      stage_fail
      die "could not install man pages"
    fi
  fi

  # ── Completions ─────────────────────────────────────────────────────────────
  if [ -d "completions" ]; then
    stage "⌨️  shell completions"
    if install_completions; then
      stage_ok
    else
      stage_fail
      die "could not install shell completions"
    fi
  fi

  # ── systemd user unit ───────────────────────────────────────────────────────
  local systemd_unit=""
  if [ "${OS}" = "linux" ] && [ -f "systemd/gtmd.service" ]; then
    stage "⚙️  systemd user unit"
    mkdir -p "${systemd_dir}"
    if install -m 0644 "systemd/gtmd.service" "${systemd_dir}/gtmd.service"; then
      systemd_unit="${systemd_dir}/gtmd.service"
      stage_ok
    else
      stage_fail
      die "could not install the systemd unit"
    fi
  fi

  # ── Desktop entry + icon ────────────────────────────────────────────────────
  if [ -f "desktop/gtm.desktop" ]; then
    stage "🖥️  desktop entry"
    mkdir -p "${applications_dir}"
    if install -m 0644 "desktop/gtm.desktop" "${applications_dir}/gtm.desktop"; then
      stage_ok
    else
      stage_fail
      die "could not install the desktop entry"
    fi
  fi
  if [ -f "icons/gtm.svg" ]; then
    stage "🎨 icon"
    mkdir -p "${icons_dir}"
    if install -m 0644 "icons/gtm.svg" "${icons_dir}/gtm.svg"; then
      stage_ok
    else
      stage_fail
      die "could not install the icon"
    fi
  fi

  ok "installation complete"

  # Everything is in place: offer to enable and start the daemon. Interactive
  # terminals only; `-y` / non-interactive runs skip this without enabling.
  if [ -n "${systemd_unit}" ] && [ "${ASSUME_YES}" != 1 ] && [ -t 0 ] \
    && command -v systemctl >/dev/null 2>&1; then
    local reply=""
    printf "${BOLD}Enable and start the gtm daemon now? [y/N] ${NC}" >&2
    read -r reply || reply=""
    case "${reply}" in
      [yY] | [yY][eE][sS])
        systemctl --user daemon-reload 2>/dev/null || true
        if systemctl --user enable --now gtmd 2>/dev/null; then
          ok "gtmd enabled and started"
        else
          fail "could not enable gtmd"
        fi
        ;;
      *) ;;
    esac
  fi

  if ! echo ":${PATH}:" | grep -q ":${bindir}:"; then
    info "${bindir} is not in your \$PATH — adding it to your shell profiles..."

    # bash: ~/.bashrc
    if [ -f "${HOME}/.bashrc" ] && ! grep -q "export PATH=.*${bindir//\//\\/}" "${HOME}/.bashrc" 2>/dev/null; then
      echo "" >> "${HOME}/.bashrc"
      echo "# Added by gtm installer" >> "${HOME}/.bashrc"
      echo "export PATH=\"${bindir}:\$PATH\"" >> "${HOME}/.bashrc"
      ok "added PATH to ~/.bashrc"
    fi

    # zsh: ~/.zshrc
    if [ -f "${HOME}/.zshrc" ] && ! grep -q "export PATH=.*${bindir//\//\\/}" "${HOME}/.zshrc" 2>/dev/null; then
      echo "" >> "${HOME}/.zshrc"
      echo "# Added by gtm installer" >> "${HOME}/.zshrc"
      echo "export PATH=\"${bindir}:\$PATH\"" >> "${HOME}/.zshrc"
      ok "added PATH to ~/.zshrc"
    fi

    # fish: ~/.config/fish/config.fish
    local fish_config="${HOME}/.config/fish/config.fish"
    if [ -f "${fish_config}" ] && ! grep -q "fish_add_path ${bindir//\//\\/}" "${fish_config}" 2>/dev/null; then
      echo "" >> "${fish_config}"
      echo "# Added by gtm installer" >> "${fish_config}"
      echo "fish_add_path ${bindir}" >> "${fish_config}"
      ok "added PATH to ~/.config/fish/config.fish"
    elif [ ! -f "${fish_config}" ]; then
      mkdir -p "${HOME}/.config/fish"
      echo "# Added by gtm installer" > "${fish_config}"
      echo "fish_add_path ${bindir}" >> "${fish_config}"
      ok "added PATH to ~/.config/fish/config.fish"
    fi

    info "Restart your shell or source your shell profile to pick up the PATH change"
  fi
}

# Install every man page in man/man1/ into ${mandir}. Exits non-zero on error
# so the surrounding stage can print its ✘ marker.
install_man_pages() {
  local f
  mkdir -p "${mandir}" || return 1
  for f in man/man1/*.1; do
    [ -f "${f}" ] || continue
    install -m 0644 "${f}" "${mandir}/$(basename "${f}")" || return 1
  done
}

# Place each completion file into the conventional directory for its shell.
# Exits non-zero on error so the surrounding stage can print its ✘ marker.
install_completions() {
  local f base
  for f in completions/*; do
    [ -f "${f}" ] || continue
    base="$(basename "${f}")"
    case "${base}" in
      gtm.bash | gtmd.bash)
        mkdir -p "${bash_comp_dir}" || return 1
        install -m 0644 "${f}" "${bash_comp_dir}/${base%.bash}" || return 1
        ;;
      _gtm | _gtmd)
        mkdir -p "${zsh_comp_dir}" || return 1
        install -m 0644 "${f}" "${zsh_comp_dir}/${base}" || return 1
        ;;
      gtm.fish | gtmd.fish)
        mkdir -p "${fish_comp_dir}" || return 1
        install -m 0644 "${f}" "${fish_comp_dir}/${base}" || return 1
        ;;
      gtm.elv | gtmd.elv)
        mkdir -p "${elvish_comp_dir}" || return 1
        install -m 0644 "${f}" "${elvish_comp_dir}/${base}" || return 1
        ;;
      gtm.ps1 | gtmd.ps1)
        mkdir -p "${powershell_comp_dir}" || return 1
        install -m 0644 "${f}" "${powershell_comp_dir}/${base}" || return 1
        ;;
    esac
  done
}

# ── Entry point ────────────────────────────────────────────────────────────────

if [ "${IN_ARCHIVE}" = 1 ]; then
  install_from_archive
else
  bootstrap_install "$@"
fi