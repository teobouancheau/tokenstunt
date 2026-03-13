#!/usr/bin/env bash
set -euo pipefail

REPO="teobouancheau/tokenstunt"
BIN_DIR="${CLAUDE_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}/bin"
BINARY_NAME="tokenstunt"

get_latest_version() {
  curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' \
    | head -1 \
    | sed 's/.*"tag_name": *"//;s/".*//'
}

get_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "${os}" in
    Darwin)
      case "${arch}" in
        arm64|aarch64) echo "aarch64-apple-darwin" ;;
        x86_64)        echo "x86_64-apple-darwin" ;;
        *)             echo "unsupported architecture: ${arch}" >&2; exit 1 ;;
      esac
      ;;
    Linux)
      case "${arch}" in
        aarch64|arm64) echo "aarch64-unknown-linux-gnu" ;;
        x86_64)        echo "x86_64-unknown-linux-gnu" ;;
        *)             echo "unsupported architecture: ${arch}" >&2; exit 1 ;;
      esac
      ;;
    *)
      echo "unsupported OS: ${os}" >&2
      exit 1
      ;;
  esac
}

main() {
  local version target url archive

  version="$(get_latest_version)"
  if [[ -z "${version}" ]]; then
    echo "Failed to fetch latest release version." >&2
    echo "Falling back to cargo install..." >&2
    cargo_fallback
    return
  fi

  if [[ -x "${BIN_DIR}/${BINARY_NAME}" ]]; then
    local installed
    installed="$("${BIN_DIR}/${BINARY_NAME}" --version 2>/dev/null | awk '{print $2}')"
    if [[ "${installed}" == "${version#v}" ]]; then
      return 0
    fi
    echo "Updating Token Stunt ${installed} → ${version#v}..."
    rm -f "${BIN_DIR}/${BINARY_NAME}"
  else
    echo "Installing Token Stunt..."
  fi

  target="$(get_target)"
  archive="${BINARY_NAME}-${target}.tar.gz"
  url="https://github.com/${REPO}/releases/download/${version}/${archive}"

  echo "Downloading ${version} for ${target}..."
  mkdir -p "${BIN_DIR}"

  if curl -fsSL "${url}" | tar xz -C "${BIN_DIR}"; then
    chmod +x "${BIN_DIR}/${BINARY_NAME}"
    echo "Installed ${BINARY_NAME} ${version#v} to ${BIN_DIR}/${BINARY_NAME}"
  else
    echo "Download failed. Falling back to cargo install..." >&2
    cargo_fallback
  fi
}

cargo_fallback() {
  if ! command -v cargo &>/dev/null; then
    echo "Error: cargo not found. Install Rust (https://rustup.rs) or download a release manually." >&2
    exit 1
  fi

  mkdir -p "${BIN_DIR}"
  cargo install --git "https://github.com/${REPO}" --root "${BIN_DIR%/bin}" --locked
  echo "Installed ${BINARY_NAME} via cargo to ${BIN_DIR}/${BINARY_NAME}"
}

main
