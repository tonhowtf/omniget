#!/usr/bin/env bash
# One-step setup for the omniget skill. Detects the OS, checks for the tools the
# skill needs, and installs the missing ones after a single confirmation.
#
#   setup.sh              interactive: check, confirm, install, then key setup + check
#   setup.sh --yes        install missing tools without the confirmation prompt
#   setup.sh --local      also install a local Whisper engine + default model
#   setup.sh --check-only  report status and exit; install nothing (safe for Claude)
#
# Core tools (needed for everything): yt-dlp, ffmpeg. Cloud transcription then needs
# only an OpenAI or Gemini key (see keys.sh). Local transcription (--local) additionally
# wants whisper-cli + a ggml model. Mac (brew) and Windows (winget/scoop) install
# automatically; Linux runs the distro manager when it can, else prints the commands.
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=resolve-tools.sh
. "$here/resolve-tools.sh"

ASSUME_YES=false; WANT_LOCAL=false; CHECK_ONLY=false; WANT_CLI=true; DO_UPDATE=false
for arg in "$@"; do
  case "$arg" in
    --yes|-y) ASSUME_YES=true ;;
    --local) WANT_LOCAL=true ;;
    --check-only) CHECK_ONLY=true ;;
    --no-cli) WANT_CLI=false ;;
    --update) DO_UPDATE=true; ASSUME_YES=true ;;
    *) echo "setup: unknown flag $arg" >&2; exit 2 ;;
  esac
done

OS="$(og_os)"
PM=""
detect_pm() {
  case "$OS" in
    mac) command -v brew >/dev/null 2>&1 && PM=brew ;;
    windows) for c in winget scoop choco; do command -v "$c" >/dev/null 2>&1 && { PM="$c"; break; }; done ;;
    linux) for c in apt-get dnf pacman zypper; do command -v "$c" >/dev/null 2>&1 && { PM="$c"; break; }; done ;;
  esac
}
detect_pm

if $DO_UPDATE; then
  echo "Updating tools on $OS${PM:+ (via $PM)}"
  echo
  case "$PM" in
    brew) echo ">>> brew upgrade yt-dlp ffmpeg"; brew upgrade yt-dlp ffmpeg 2>/dev/null || true ;;
    winget) echo ">>> winget upgrade yt-dlp.yt-dlp Gyan.FFmpeg"; winget upgrade --silent yt-dlp.yt-dlp 2>/dev/null; winget upgrade --silent Gyan.FFmpeg 2>/dev/null || true ;;
    scoop) echo ">>> scoop update yt-dlp ffmpeg"; scoop update yt-dlp ffmpeg 2>/dev/null || true ;;
    apt-get) command -v pipx >/dev/null 2>&1 && { echo ">>> pipx upgrade yt-dlp"; pipx upgrade yt-dlp 2>/dev/null || true; } ;;
    dnf|pacman|zypper) echo "Update yt-dlp/ffmpeg with your package manager; then re-run without --update." ;;
    *) echo "No package manager detected; update yt-dlp/ffmpeg yourself." ;;
  esac
  echo
  echo "== omniget-cli =="
  install_omniget_cli
  echo
  echo "== Transcription API keys =="
  bash "$here/keys.sh" check
  echo
  echo "Update complete."
  exit 0
fi

# install_cmd TOOL -> echoes the shell command to install TOOL with the detected PM,
# or empty if we don't have a recipe for this OS/PM.
install_cmd() {
  local tool="$1"
  case "$tool:$PM" in
    yt-dlp:brew) echo "brew install yt-dlp" ;;
    yt-dlp:winget) echo "winget install --silent --accept-package-agreements --accept-source-agreements yt-dlp.yt-dlp" ;;
    yt-dlp:scoop) echo "scoop install yt-dlp" ;;
    yt-dlp:choco) echo "choco install -y yt-dlp" ;;
    yt-dlp:apt-get) echo "sudo apt-get update && sudo apt-get install -y pipx && pipx install yt-dlp" ;;
    yt-dlp:dnf) echo "sudo dnf install -y yt-dlp" ;;
    yt-dlp:pacman) echo "sudo pacman -S --noconfirm yt-dlp" ;;
    yt-dlp:zypper) echo "sudo zypper install -y yt-dlp" ;;
    ffmpeg:brew) echo "brew install ffmpeg" ;;
    ffmpeg:winget) echo "winget install --silent --accept-package-agreements --accept-source-agreements Gyan.FFmpeg" ;;
    ffmpeg:scoop) echo "scoop install ffmpeg" ;;
    ffmpeg:choco) echo "choco install -y ffmpeg" ;;
    ffmpeg:apt-get) echo "sudo apt-get update && sudo apt-get install -y ffmpeg" ;;
    ffmpeg:dnf) echo "sudo dnf install -y ffmpeg" ;;
    ffmpeg:pacman) echo "sudo pacman -S --noconfirm ffmpeg" ;;
    ffmpeg:zypper) echo "sudo zypper install -y ffmpeg" ;;
    whisper-cli:brew) echo "brew install whisper-cpp" ;;
    *) echo "" ;;
  esac
}

missing=()
plan=()
note_missing() {
  local tool="$1"
  og_find_tool "$tool" >/dev/null 2>&1 && return
  missing+=("$tool")
  local cmd; cmd="$(install_cmd "$tool")"
  [ -n "$cmd" ] && plan+=("$cmd") || plan+=("# no auto-install recipe for $tool on $OS — see README")
}

echo "OmniGet skill setup — $OS${PM:+ (package manager: $PM)}"
echo

note_missing yt-dlp
note_missing ffmpeg
$WANT_LOCAL && note_missing whisper-cli

if [ ${#missing[@]} -eq 0 ]; then
  echo "Core tools present:"
  for t in yt-dlp ffmpeg $([ "$WANT_LOCAL" = true ] && echo whisper-cli); do
    printf '  %-12s %s\n' "$t" "$(og_tool_path "$t")"
  done
else
  echo "Missing: ${missing[*]}"
  if [ -z "$PM" ]; then
    echo
    case "$OS" in
      mac) echo "Homebrew isn't installed. Install it first (one line), then re-run setup:"; echo '  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"' ;;
      windows) echo "No winget/scoop/choco found. Install winget (App Installer from the Microsoft Store) or scoop, then re-run." ;;
      linux) echo "No supported package manager detected. Install yt-dlp and ffmpeg via your distro, then re-run." ;;
    esac
  else
    echo "Planned install commands:"
    for c in "${plan[@]}"; do echo "  $c"; done
  fi
fi
echo

OMNIGET_REPO="${OMNIGET_REPO:-tonhowtf/omniget}"

# Prebuilt omniget-cli asset triple for this machine, or empty if unsupported.
cli_triple() {
  case "$OS:$(og_arch)" in
    mac:aarch64) echo "aarch64-apple-darwin:tar.gz" ;;
    mac:x86_64) echo "x86_64-apple-darwin:tar.gz" ;;
    windows:x86_64) echo "x86_64-pc-windows-msvc:zip" ;;
    linux:x86_64) echo "x86_64-unknown-linux-gnu:tar.gz" ;;
    *) echo "" ;;
  esac
}

# Download the prebuilt omniget-cli for this OS/arch into the skill bin dir.
# omniget-cli carries OmniGet's native Instagram/X/Bilibili/Threads extractors, which
# are more reliable than plain yt-dlp for those sites.
install_omniget_cli() {
  local triple ext tag ver asset url dest bindir tmp
  triple="$(cli_triple)"
  if [ -z "$triple" ]; then
    echo "  omniget-cli: no prebuilt binary for $OS/$(og_arch) — the skill will use yt-dlp for every site."
    return 1
  fi
  ext="${triple#*:}"; triple="${triple%%:*}"
  tag="$(curl -fsSL -m 30 "https://api.github.com/repos/$OMNIGET_REPO/releases/latest"         | python3 -c 'import json,sys; print(json.load(sys.stdin).get("tag_name",""))' 2>/dev/null)"
  [ -n "$tag" ] || { echo "  omniget-cli: couldn't resolve the latest release (offline?). Skipping."; return 1; }
  ver="${tag#v}"
  asset="omniget-cli-$ver-$triple.$ext"
  url="https://github.com/$OMNIGET_REPO/releases/download/$tag/$asset"
  bindir="$(og_skill_bin_dir)"; mkdir -p "$bindir"
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/omniget-cli.XXXXXX")"
  echo "  omniget-cli: downloading $asset ..."
  if ! curl -fSL --progress-bar -o "$tmp/pkg" "$url"; then
    echo "  omniget-cli: download failed ($url)"; rm -rf "$tmp"; return 1
  fi
  ( cd "$tmp" && case "$ext" in tar.gz) tar xzf pkg ;; zip) unzip -oq pkg ;; esac )
  local found; found="$(find "$tmp" -type f \( -name omniget-cli -o -name omniget-cli.exe \) | head -1)"
  if [ -z "$found" ]; then echo "  omniget-cli: binary not found in archive"; rm -rf "$tmp"; return 1; fi
  dest="$bindir/$(basename "$found")"
  mv "$found" "$dest"; chmod +x "$dest"
  [ "$OS" = mac ] && xattr -d com.apple.quarantine "$dest" 2>/dev/null
  rm -rf "$tmp"
  if "$dest" --version >/dev/null 2>&1; then
    echo "  omniget-cli: installed ($tag) -> $dest"
  else
    echo "  omniget-cli: installed but did not run cleanly; the skill will fall back to yt-dlp"
  fi
}

run_installs() {
  [ ${#missing[@]} -eq 0 ] && return 0
  [ -z "$PM" ] && { echo "Can't auto-install without a package manager (see above)."; return 1; }
  local i=0 tool cmd rc=0
  for tool in "${missing[@]}"; do
    cmd="${plan[$i]}"; i=$((i+1))
    case "$cmd" in \#*) echo "Skipping $tool: $cmd"; continue ;; esac
    echo ">>> $cmd"
    if printf '%s' "$cmd" | grep -q 'sudo ' && [ ! -t 0 ]; then
      echo "    needs sudo and there's no terminal here — run this line yourself:"
      echo "    $cmd"
      rc=1; continue
    fi
    if bash -c "$cmd"; then
      og_find_tool "$tool" >/dev/null 2>&1 && echo "    ok: $tool -> $(og_tool_path "$tool")" || echo "    installed, but $tool still not on PATH — open a new shell and re-check"
    else
      echo "    failed to install $tool"; rc=1
    fi
  done
  return $rc
}

if [ ${#missing[@]} -gt 0 ] && [ "$CHECK_ONLY" != true ]; then
  do_install=false
  if $ASSUME_YES; then
    do_install=true
  elif [ -t 0 ]; then
    printf 'Install the %d missing tool(s) now? [y/N] ' "${#missing[@]}"
    read -r ans
    case "$ans" in y|Y|yes|YES) do_install=true ;; esac
  else
    echo "Re-run with --yes to install, or run the commands above yourself."
  fi
  $do_install && run_installs
  echo
fi

if $WANT_CLI && [ "$CHECK_ONLY" != true ] && ! og_find_tool omniget-cli >/dev/null 2>&1; then
  echo "== omniget-cli (native Instagram / X / Bilibili / Threads extractors) =="
  install_omniget_cli
  echo
fi

if $WANT_LOCAL && og_find_tool whisper-cli >/dev/null 2>&1 && ! og_whisper_model >/dev/null 2>&1; then
  echo "Local Whisper is installed but has no model."
  if [ "$CHECK_ONLY" != true ] && { $ASSUME_YES || [ -t 0 ]; }; then
    if $ASSUME_YES || { printf 'Download the default model (large-v3-turbo-q5_0, 547 MB)? [y/N] '; read -r m; case "$m" in y|Y|yes|YES) true ;; *) false ;; esac; }; then
      "$here/get-model.sh" large-v3-turbo-q5_0 && echo "  model ready"
    fi
  else
    echo "  get one with:  bash \"$here/get-model.sh\" large-v3-turbo-q5_0"
  fi
  echo
fi

# Transcription keys: report status, and point at the interactive setter (never run by Claude).
echo "== Transcription API keys =="
bash "$here/keys.sh" check
echo
echo "To add or change a key, run this in YOUR terminal (input is hidden, keys never leave your machine):"
echo "  bash \"$here/keys.sh\" set"
echo
echo "Setup check complete. Run  bash \"$here/doctor.sh\"  any time for the full picture."
