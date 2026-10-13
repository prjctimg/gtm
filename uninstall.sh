#!/usr/bin/env bash
# gtm uninstaller — see https://gtmd.dev
#
# Usage:
#   curl -fsSL https://gtmd.dev/uninstall.sh | bash
#   uninstall.sh                      # remove what install.sh put on this system
#   uninstall.sh --purge              # also delete config, library and caches
#   uninstall.sh --prefix /usr/local  # uninstall from a custom prefix
#
# Directory resolution is the same as install.sh's, env var for env var, so
# `install.sh --prefix ~/.local && uninstall.sh --prefix ~/.local` always agree.
# Nothing is removed unless it is a file the installer owns: every path is
# checked against the expected gtm names first, and `--purge` is the only way
# to touch a directory as a whole.
#
# Recognised standard environment variables (all overridable):
#   PREFIX DATAROOTDIR DATADIR BINDIR MANDIR SYSTEMD_DIR APPLICATIONS_DIR
#   ICONS_DIR XDG_DATA_HOME XDG_CONFIG_HOME XDG_CACHE_HOME ZDOTDIR
#   BASH_COMPLETION_DIR ZSH_COMPLETION_DIR FISH_COMPLETION_DIR
#   ELVISH_COMPLETION_DIR POWERSHELL_COMPLETION_DIR

set -euo pipefail

NC='\033[0m'
MUTED='\033[0;2m'
RED='\033[0;31m'
GREEN='\033[0;32m'
BOLD='\033[1m'

usage() {
  cat <<EOF
gtm uninstaller — https://gtmd.dev

Usage: uninstall.sh [options]

Options:
  -h, --help            Show this help message
  -p, --prefix <dir>    Prefix gtm was installed under (default: \$HOME/.local)
      --purge           Also remove config, library and caches (asks first)
  -y, --yes             Non-interactive: never prompt

Examples:
  curl -fsSL https://gtmd.dev/uninstall.sh | bash
  uninstall.sh --purge
  uninstall.sh --prefix /usr/local
EOF
}

# Logging and stage helpers, matching install.sh's output shape.
info() { printf "${MUTED}%s${NC}\n" "$*" >&2; }
log() { printf "${NC}%s\n" "$*" >&2; }
ok() { printf "${GREEN}✔${NC} %s\n" "$*" >&2; }
fail() { printf "${RED}✘${NC} %s\n" "$*" >&2; }
die() {
  printf "${RED}%s${NC}\n" "$*" >&2
  exit 1
}
stage() { printf "${BOLD}%s${NC}" "$*" >&2; }
stage_ok() { printf " ${GREEN}✔${NC}\n" >&2; }
stage_fail() { printf " ${RED}✘${NC}\n" >&2; }

PREFIX="${PREFIX:-$HOME/.local}"
PURGE=0
ASSUME_YES=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h | --help)
      usage
      exit 0
      ;;
    --purge)
      PURGE=1
      shift
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

datarootdir="${DATAROOTDIR:-${PREFIX}/share}"
datadir="${DATADIR:-${datarootdir}}"
bindir="${BINDIR:-${PREFIX}/bin}"
mandir="${MANDIR:-${datadir}/man/man1}"

if [ "${PREFIX#"$HOME"}" != "${PREFIX}" ]; then
  xdg_data="${XDG_DATA_HOME:-$HOME/.local/share}"
  xdg_config="${XDG_CONFIG_HOME:-$HOME/.config}"
else
  xdg_data="${datarootdir}"
  xdg_config="${datarootdir}"
fi

systemd_dir="${SYSTEMD_DIR:-${xdg_data}/systemd/user}"
bash_comp_dir="${BASH_COMPLETION_DIR:-${xdg_data}/bash-completion/completions}"
zsh_comp_dir="${ZSH_COMPLETION_DIR:-${ZDOTDIR:-$HOME}/.zsh/completions}"
fish_comp_dir="${FISH_COMPLETION_DIR:-${xdg_config}/fish/completions}"
elvish_comp_dir="${ELVISH_COMPLETION_DIR:-$HOME/.elvish/lib}"
powershell_comp_dir="${POWERSHELL_COMPLETION_DIR:-${xdg_data}/powershell/Modules}"
applications_dir="${APPLICATIONS_DIR:-${datadir}/applications}"
icons_dir="${ICONS_DIR:-${datadir}/icons/hicolor/scalable/apps}"

# Remove <path> when it exists, reporting the basename rather than the whole
# path: the list is long and the prefix on every line is noise. A missing file
# is not a failure — uninstalling something half-installed has to work.
removed=0
rm_path() {
  local path="$1" what="${2:-}"
  [ -e "$path" ] || [ -L "$path" ] || return 0
  what="${what:-$(basename "$path")}"
  if rm -rf "$path"; then
    ok "removed ${what}"
    removed=$((removed + 1))
  else
    fail "could not remove ${what} (${path})"
  fi
}

# Remove several files under a directory in one stage. Skips the stage heading
# entirely when none of them exist, so an uninstall of a subset does not print a
# row of empty ✔s.
rm_files() {
  local dir="$1" label="$2"
  shift 2
  local f hit=0
  for f in "$@"; do
    if [ -e "${dir}/${f}" ]; then
      hit=1
      break
    fi
  done
  [ "${hit}" = 1 ] || return 0
  stage "${label}"
  for f in "$@"; do
    rm_path "${dir}/${f}"
  done
  stage_ok
}

# ── Daemon ────────────────────────────────────────────────────────────────────
# Disabled and stopped before its unit file is deleted: a running daemon holds
# the socket the client reconnects to, and removing a unit out from under a
# running service leaves systemd complaining about a unit it cannot find.
# `is-enabled` exits 0 even for `disabled`, so the state word is what is read.
if command -v systemctl >/dev/null 2>&1; then
  enabled="$(systemctl --user is-enabled gtmd.service 2>/dev/null || true)"
  active="$(systemctl --user is-active gtmd.service 2>/dev/null || true)"
  if [ "${active}" = "active" ] || { [ -n "${enabled}" ] && [ "${enabled}" != "disabled" ] && [ "${enabled}" != "not-found" ]; }; then
    stage "⚙️  stopping gtmd"
    systemctl --user disable --now gtmd.service >/dev/null 2>&1 || true
    stage_ok
    ok "gtmd stopped and disabled"
  fi
fi

# A daemon started by hand (not via systemd) holds the socket and the mixer;
# both are released on SIGTERM.
if command -v pgrep >/dev/null 2>&1 && pgrep -x gtmd >/dev/null 2>&1; then
  stage "⏹️  stopping a running gtmd"
  command -v pkill >/dev/null 2>&1 && pkill -TERM -x gtmd 2>/dev/null || :
  stage_ok
  ok "gtmd stopped"
fi

# ── Installed files ───────────────────────────────────────────────────────────
stage "📦 binaries"
rm_path "${bindir}/gtm" "gtm"
rm_path "${bindir}/gtmd" "gtmd"
stage_ok

rm_files "${mandir}" "📖 man pages" gtm.1 gtmd.1 gtmd-ipc.1

rm_files "${bash_comp_dir}" "⌨️  shell completions" gtm gtmd
rm_files "${zsh_comp_dir}" "⌨️  shell completions" _gtm _gtmd
rm_files "${fish_comp_dir}" "⌨️  shell completions" gtm.fish gtmd.fish
rm_files "${elvish_comp_dir}" "⌨️  shell completions" gtm.elv gtmd.elv
rm_files "${powershell_comp_dir}" "⌨️  shell completions" gtm.ps1 gtmd.ps1

if [ -f "${systemd_dir}/gtmd.service" ]; then
  stage "⚙️  systemd user unit"
  rm_path "${systemd_dir}/gtmd.service" "gtmd.service"
  systemctl --user daemon-reload >/dev/null 2>&1 || true
  stage_ok
fi

rm_path "${applications_dir}/gtm.desktop" "gtm.desktop"
rm_path "${icons_dir}/gtm.svg" "gtm.svg"

# ── Shell profile PATH lines ──────────────────────────────────────────────────
# Only the lines the installer wrote: its marker comment, and the PATH line
# beside it. The bin directory is matched in all three forms a line can spell
# it — expanded, `$HOME`-relative and `~`-relative — because the installer
# writes the first and a hand-edited profile usually has the other two.
# Anything else in the file belongs to the user.
bindir_tail="${bindir#"$HOME"}"
stage "📝 shell profile"
stripped=0
for file in "${HOME}/.bashrc" "${HOME}/.zshrc" "${HOME}/.config/fish/config.fish"; do
  [ -f "$file" ] || continue
  grep -q "Added by gtm installer" "$file" 2>/dev/null || continue
  tmp="$(mktemp)"
  grep -vE "Added by gtm installer|${bindir}|\\\$HOME${bindir_tail}|~${bindir_tail}" "$file" \
    | sed '/./,$!d' >"$tmp" && cat "$tmp" >"$file"
  rm -f "$tmp"
  ok "cleaned $(basename "$file")"
  stripped=$((stripped + 1))
done
[ "${stripped}" -gt 0 ] || info "no installer-written PATH entries found"
stage_ok

# ── Config, library and caches ───────────────────────────────────────────────
if [ "${PURGE}" = 1 ]; then
  reply=""
  if [ "${ASSUME_YES}" = 1 ]; then
    reply="y"
  elif [ -t 0 ]; then
    printf "${BOLD}Also remove config, library and caches? [y/N] ${NC}" >&2
    read -r reply || reply=""
  fi
  case "${reply}" in
    [yY] | [yY][eE][sS]) ;;
    *) PURGE=0 ;;
  esac
fi

if [ "${PURGE}" = 1 ]; then
  stage "🗑️  config, library and caches"
  rm_path "${XDG_CONFIG_HOME:-$HOME/.config}/gtm" "config"
  rm_path "${XDG_DATA_HOME:-$HOME/.local/share}/gtm" "library, state and covers"
  rm_path "${XDG_CACHE_HOME:-$HOME/.cache}/gtm" "cover cache"
  # The runtime directory holds the daemon's socket and pid file. A running
  # daemon is already gone by this point (both the systemd and the pkill stage
  # above), so the leftovers are stale sockets — but if one is somehow still
  # holding the file, leave it: deleting a live daemon's socket breaks the
  # client with no way back short of a restart.
  if [ -n "${XDG_RUNTIME_DIR:-}" ] \
    && ! { command -v pgrep >/dev/null 2>&1 && pgrep -x gtmd >/dev/null 2>&1; }; then
    rm_path "${XDG_RUNTIME_DIR}/gtm" "runtime files"
  fi
  stage_ok
fi

if [ "${removed}" -gt 0 ]; then
  ok "uninstalled"
  info "reinstall with: curl -fsSL https://gtmd.dev/install.sh | bash"
else
  log "nothing to remove — gtm does not appear to be installed under ${PREFIX}"
fi
