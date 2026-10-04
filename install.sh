#!/usr/bin/env bash
# gtm installer — see https://github.com/prjctimg/gtm
#
# Usage:
#   curl -fsSL https://gtmd.dev/install.sh | bash
#   install.sh                        # download and install the release for this system
#   install.sh --version 0.2.83       # pin a specific release
#   install.sh --nightly              # install the latest nightly prerelease
#   install.sh --prefix ~/.local      # install under a custom prefix
#
# When run from inside a gtm release archive (bin/gtm + bin/gtmd sit next to
# this script) the same file installs the bundled assets directly. The
# standalone form is a thin bootstrap: it downloads the per-platform archive,
# extracts it, and installs from it — using itself, not the copy inside the
# archive, so that installer changes do not wait for a release. The installer
# logic still lives in a single file that is shipped in every archive, for the
# case where someone downloads an archive and runs ./install.sh directly.
#
# Recognised standard environment variables (all overridable):
#   PREFIX DATAROOTDIR DATADIR BINDIR MANDIR SYSTEMD_DIR APPLICATIONS_DIR
#   ICONS_DIR XDG_DATA_HOME XDG_CONFIG_HOME ZDOTDIR BASH_COMPLETION_DIR
#   ZSH_COMPLETION_DIR FISH_COMPLETION_DIR ELVISH_COMPLETION_DIR
#   POWERSHELL_COMPLETION_DIR

set -euo pipefail

REPO="prjctimg/gtm"

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
  -v, --version <ver>   Install a specific version (e.g. 0.2.83)
      --nightly         Install the latest nightly prerelease
  -p, --prefix <dir>    Install prefix for the tarball (default: \$HOME/.local)
  -y, --yes             Non-interactive: never prompt (e.g. to enable gtmd)

When run from inside a release archive this file installs the bundled
binaries, man pages, completions, systemd unit, desktop entry and icon.

Examples:
  curl -fsSL https://gtmd.dev/install.sh | bash
  install.sh --version 0.2.83
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

# Every network call gets these. Without them a slow DNS answer, a captive
# portal or a half-open connection leaves curl on the socket with nothing
# printed and nothing to interrupt it with — the script looks hung, and there
# is no way to tell that apart from a slow download. The retries cover the two
# cases that are transient rather than fatal: a connection refused while the
# network comes up, and a 5xx/429 from the API while a release is mid-publish.
#
# `--retry-connrefused` needs curl 7.52 (2016). An older curl rejects the whole
# command rather than ignoring the flag, so it is added only when supported.
curl_supports() {
  curl --help all 2>/dev/null | grep -q -- "$1"
}

CURL_WAIT=(--silent --show-error --connect-timeout 10 --max-time 60 --retry 2 --retry-delay 2)
if curl_supports "retry-connrefused"; then
  CURL_WAIT+=(--retry-connrefused)
fi

# Fetch a small JSON document, failing quietly. Callers decide what a failure
# means — the stable path falls back to a conventional URL, the nightly path
# aborts — so this only has to be bounded and loud enough to debug.
api_get() {
  curl "${CURL_WAIT[@]}" -fsSL "$1"
}

# The size of <url> in bytes, or 0 when the server will not say.
#
# One extra round trip, and only so the percentage can be honest. Without a
# total there is nothing to divide by, and a percentage of an unknown total is
# not a percentage. Header names are matched with explicit classes rather than
# `IGNORECASE`, which is a GNU awk extension and this installer also runs under
# busybox awk on Alpine and Termux.
remote_size() {
  curl -fsSLI --connect-timeout 10 --max-time 30 "$1" 2>/dev/null \
    | awk '/^[Cc]ontent-[Ll]ength:/ { gsub(/\r/, "", $2); n = $2 } END { print n + 0 }'
}

# Seconds a download may receive no new bytes at all before it is called dead.
#
# Measured in bytes, not in average rate, and that distinction is the whole
# point: a slow link that keeps trickling is slow, not dead. This one averages
# 35 kB/s while pausing for tens of seconds at a stretch, and a rate floor
# (`--speed-limit`) added for the same purpose killed that exact download four
# times over — each curl retry restarts from zero, so it could never finish.
# Ninety seconds with not one new byte is a connection nobody is reading.
DOWNLOAD_STALL_SECS=90

# Download a URL, reporting a percentage on a terminal.
#
# curl's own `--progress-bar` is gone: it draws a row of `#`, `=` and `O`
# glyphs, which is the one thing in this output that does not read as text. The
# percentage is computed here instead — bytes on disk against the size the
# server reported — so it is a real number rather than curl's guess at one.
#
# The bytes arrive through a backgrounded curl because there is no way to ask a
# foreground curl what it has written so far. On a non-terminal stderr (CI
# logs, `2>log`, `| tee`) there is nothing to redraw the line into, so no
# progress is printed — but the stall guard still runs, because a CI log that
# stops forever is the same bug as a terminal that does. curl's errors come
# through either way, which the old `2>/dev/null` swallowed along with them: a
# 404 used to print "download failed" and not "404", which is the one line that
# says why.
#
# No `--max-time`, because an 18 MB archive on a slow link legitimately takes
# minutes and a total cap would fail the installs that most need patience.
#   download_simple <url> <outfile>
download_simple() {
  local url="$1" out="$2"
  local curl_common=(-fL --silent --show-error --connect-timeout 15
    --retry 3 --retry-delay 2)

  local tty=0
  if [ -t 2 ]; then tty=1; fi

  local total pid have prev=-1 still=0 last=-1 code=0 killed=0
  total="$(remote_size "$url")"
  curl "${curl_common[@]}" "$url" -o "$out" &
  pid=$!
  while kill -0 "$pid" 2>/dev/null; do
    # The file does not exist yet on the first poll — curl has not been
    # scheduled — and `wc -c < missing` is a redirection failure the *shell*
    # reports, which lands on this line and corrupts the percentage beside it.
    # Ask whether it is there first.
    if [ -f "$out" ]; then
      have="$(wc -c < "$out" 2>/dev/null || echo 0)"
      have="${have:-0}"
    else
      have=0
    fi

    if [ "${have}" -gt "${prev}" ]; then
      still=0
    else
      still=$((still + 1))
      if [ "${still}" -ge "${DOWNLOAD_STALL_SECS}" ]; then
        kill "${pid}" 2>/dev/null || true
        killed=1
        break
      fi
    fi
    prev="${have}"

    if [ "${tty}" -eq 1 ]; then
      if [ "${total}" -gt 0 ] 2>/dev/null; then
        local pct=$((have * 100 / total))
        if [ "${pct}" -gt 100 ]; then pct=100; fi
        # A retry restarts the transfer and the file shrinks under us, so the
        # number only ever goes up. A percentage that goes backwards
        # mid-download reads as a fault in the installer rather than as the
        # retry it is.
        if [ "${pct}" -lt "${last}" ]; then pct="${last}"; fi
        if [ "${pct}" -ne "${last}" ]; then
          # Right-padded to a fixed width so a shorter number cannot leave the
          # tail of the previous one on screen.
          printf "${MUTED}📥 Downloading: %3d%%   ${NC}\r" "$pct" >&2
          last="${pct}"
        fi
      elif [ "${last}" -lt 0 ]; then
        # No total to divide by: say what is happening, invent nothing.
        printf "${MUTED}📥 Downloading…${NC}\r" >&2
        last=0
      fi
    fi
    sleep 1
  done
  wait "${pid}" 2>/dev/null || code=$?

  if [ "${tty}" -eq 1 ]; then
    # Erase the progress line so the next one is not written over it.
    printf '\r%*s\r' 24 '' >&2
  fi
  # 28 is curl's own "timed out", so the caller's failure message reads the same
  # whether the stall was caught here or by curl.
  if [ "${killed}" -eq 1 ]; then
    return 28
  fi
  return "${code}"
}

VERSION=""
CHANNEL="stable"
PREFIX="${PREFIX:-$HOME/.local}"
ASSUME_YES=0
# Set by `resolve_asset_url`. Script scope, like `BOOTSTRAP_TMPDIR`, so the
# name survives the call and `set -u` still sees a value.
ASSET_URL=""
RELEASE_NAME=""

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
  tag="$(api_get "https://api.github.com/repos/${REPO}/releases/latest" \
    | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p' || true)"
  [ -n "${tag}" ] || die "could not reach the GitHub API for the latest release (offline, blocked, or rate limited — try: install.sh --version <ver>)"
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
#
# `-L` is load-bearing: the repository was renamed, and the API answers the
# old name with a 301 to the new one. Without it every lookup reads as an empty
# release, which the stable path absorbed via its URL fallback but the nightly
# path reported as "not published yet" however many builds had succeeded.
#
# Sets ASSET_URL and RELEASE_NAME instead of printing the URL: a `$(...)` capture
# would run this in a subshell, where the release name — the only source of the
# nightly's commit — is discarded.
#   resolve_asset_url <tag> <archive> [strict]
resolve_asset_url() {
  local tag="$1" archive="$2" strict="${3:-0}"
  local direct="https://github.com/${REPO}/releases/download/${tag}/${archive}"
  local body head
  body="$(api_get "https://api.github.com/repos/${REPO}/releases/tags/${tag}" 2>/dev/null)" || body=""
  # A release shares its `"name"` key with every asset it carries, but the assets
  # all sit inside the `assets` array, so cutting the body at that key leaves the
  # release's own name as the only match.
  head="${body%%\"assets\"*}"
  RELEASE_NAME="$(printf '%s' "${head}" | sed -n 's/.*"name": *"\([^"]*\)".*/\1/p' | head -1)"
  if printf '%s' "${body}" | grep -qF "\"name\": \"${archive}\""; then
    ASSET_URL="${direct}"
    return 0
  fi
  if [ "${strict}" = 1 ]; then
    return 1
  fi
  ASSET_URL="${direct}"
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
    # Announced, because this is the first thing the script does that can wait
    # on a network and it used to do it in silence.
    stage "🔎 resolving the latest stable release"
    VERSION="$(resolve_latest_stable_tag)"
    tag="v${VERSION}"
    stage_ok
    info "latest stable: v${VERSION}"
  fi

  local archive_name="gtm-${PLATFORM}.tar.gz"
  stage "🔎 resolving release assets"
  if [ "${CHANNEL}" = "nightly" ]; then
    # Resolve strictly against the published nightly so a draft (mid-build)
    # resolves to a clear "try again" instead of a dead 404 URL.
    resolve_asset_url "${tag}" "${archive_name}" 1 || {
      stage_fail
      die "nightly archive '${archive_name}' is not published yet — the latest nightly build may still be running or failed. Retry in a few minutes, or install a stable release with: install.sh --version <ver>"
    }
  else
    resolve_asset_url "${tag}" "${archive_name}"
  fi
  stage_ok
  local url="${ASSET_URL}"

  # Name the build rather than the file. The platform triple is already decided
  # by the machine this runs on and says nothing to the person reading it, while
  # the version is the one fact that tells two installs apart — and on a nightly,
  # where the version barely moves, only the commit does.
  local label sha=""
  if [ "${CHANNEL}" = "nightly" ]; then
    # Nightly releases are named `<version>+<short sha>+nightly`.
    sha="$(printf '%s' "${RELEASE_NAME}" | sed -n 's/.*+\([0-9a-f]\{7,\}\)+.*/\1/p')"
    label="gtm (nightly at ${sha:-unidentified build})"
  else
    label="gtm (${tag#v})"
  fi

  # Script-scope on purpose (no `local`): the EXIT trap must still read it
  # after this function returns, or `set -u` would trip on an unbound
  # variable. Uniquely named so it can never collide with a caller-exported
  # `$tmp`, and the trap uses `:?` so an empty value fails loudly instead of
  # running `rm -rf ""`.
  BOOTSTRAP_TMPDIR="$(mktemp -d)" || die "mktemp failed"
  # EXIT alone does not fire on a signal: bash runs it on a normal exit and on
  # `exit`, but a Ctrl-C during the download killed the script outright and left
  # the partial archive in /tmp. These turn the signal into an exit so the EXIT
  # trap still runs, and so the shell reports the interruption rather than
  # reporting it as whatever the last command happened to be.
  trap 'exit 130' INT
  trap 'exit 143' TERM
  trap 'rm -rf "${BOOTSTRAP_TMPDIR:?}"' EXIT

  log "⬇️  ${label}"
  if ! download_simple "${url}" "${BOOTSTRAP_TMPDIR}/${archive_name}"; then
    die "download failed: ${url}"
  fi
  # No label here: the line above already named the build, and repeating the
  # commit hash on success says nothing the reader does not have.
  ok "downloaded gtm"

  # Announced because extracting a multi-megabyte archive is not instant and
  # used to print nothing while it happened — a gap between two ✔ lines with
  # nothing in it, which reads as a stall.
  stage "📂 extracting"
  tar -xzf "${BOOTSTRAP_TMPDIR}/${archive_name}" -C "${BOOTSTRAP_TMPDIR}" || {
    stage_fail
    die "could not extract ${archive_name} (truncated download?)"
  }
  stage_ok

  local extracted_dir="${BOOTSTRAP_TMPDIR}/${archive_name%.tar.gz}"
  [ -d "${extracted_dir}" ] || die "archive did not extract correctly"

  # Install from the extracted archive using *this* script rather than the
  # copy bundled in the archive. The two diverge: each release freezes
  # install.sh at its own commit, so re-exec'ing the archived copy pins a
  # stable install to the output format and behaviour of whatever release it
  # came from, and no installer change reaches stable users until the next
  # release. The archive is only a source of assets here (`install_from_archive`
  # reads bin/, man/ and the rest relative to the current directory), and the
  # options were already parsed above, so nothing is lost by not re-exec'ing.
  (
    cd "${extracted_dir}"
    install_from_archive
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
        # `enable --now` blocks until the unit has started or given up, and the
        # give-up is 90 seconds of systemd's own default — spent with stderr
        # discarded and nothing else in flight, so on a slow session bus the
        # installer looked hung after everything was already installed. Bounded
        # here and announced, so the worst case is a clear message rather than
        # a silence. `timeout` is used only when present; the systemd block is
        # Linux-only but the tool is not always installed.
        stage "⚙️  enabling and starting gtmd (up to 30s)"
        local systemctl_enable=(systemctl --user enable --now gtmd)
        if command -v timeout >/dev/null 2>&1; then
          systemctl_enable=(timeout --kill-after=5 30 systemctl --user enable --now gtmd)
        fi
        if "${systemctl_enable[@]}" 2>/dev/null; then
          stage_ok
          ok "gtmd enabled and started"
        else
          stage_fail
          fail "could not enable gtmd — start it yourself with: systemctl --user enable --now gtmd"
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