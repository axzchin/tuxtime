#!/usr/bin/env sh
# tuxtime one-line installer.
#
# Downloads the prebuilt binary for the current Linux architecture, verifies
# its SHA-256 checksum, and installs it to ~/.local/bin (override with
# TUXXTIME_INSTALL_DIR). Targets Linux / WSL; macOS users should use
# `brew install tuxtime`.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/axzchin/tuxtime/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/axzchin/tuxtime/main/install.sh | sh -s -- v2026.7.1

set -eu

REPO="axzchin/tuxtime"
BIN="tuxtime"
DEST="${TUXXTIME_INSTALL_DIR:-$HOME/.local/bin}"

# Fail fast on missing prerequisites with a clear message.
for dep in curl tar; do
  if ! command -v "$dep" >/dev/null 2>&1; then
    echo "tuxtime: need '$dep' to install (install it first)" >&2
    exit 1
  fi
done

# Resolve the version: an explicit argument, else the latest release tag.
VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  resolved="$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest" | sed 's#.*/##')" || resolved=""
  VERSION="$resolved"
fi
# Accept "2026.7.1" as well as "v2026.7.1".
case "$VERSION" in
  v*) ;;
  *) VERSION="v${VERSION}" ;;
esac
if [ -z "$VERSION" ] || [ "$VERSION" = "v" ]; then
  echo "tuxtime: could not determine the latest release" >&2
  exit 1
fi

# Map the machine to a release target.
case "$(uname -s)/$(uname -m)" in
  Linux/x86_64 | Linux/amd64) TARGET="x86_64-unknown-linux-gnu" ;;
  Linux/aarch64 | Linux/arm64) TARGET="aarch64-unknown-linux-gnu" ;;
  *)
    echo "tuxtime: no prebuilt binary for $(uname -s)/$(uname -m)" >&2
    echo "  macOS: brew install tuxtime" >&2
    echo "  See https://github.com/${REPO}/releases/latest" >&2
    exit 1
    ;;
esac

# Download into a fresh temp dir and clean it up on exit.
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cd "$TMP"

BASE="https://github.com/${REPO}/releases/download/${VERSION}/tuxtime-${VERSION}-${TARGET}"
echo "tuxtime: downloading ${VERSION} for ${TARGET}..."
curl -fsSL -o archive.tar.gz "${BASE}.tar.gz"
curl -fsSL -o archive.tar.gz.sha256 "${BASE}.tar.gz.sha256"

# Verify the checksum before touching anything.
if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL="$(sha256sum archive.tar.gz | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  ACTUAL="$(shasum -a 256 archive.tar.gz | awk '{print $1}')"
else
  echo "tuxtime: need sha256sum or shasum to verify the download" >&2
  exit 1
fi
EXPECTED="$(awk '{print $1}' archive.tar.gz.sha256)"
if [ "$ACTUAL" != "$EXPECTED" ]; then
  echo "tuxtime: checksum mismatch" >&2
  echo "  expected: $EXPECTED" >&2
  echo "  actual:   $ACTUAL" >&2
  exit 1
fi

tar xzf archive.tar.gz
EXE="tuxtime-${VERSION}-${TARGET}/${BIN}"

# Sanity-check the downloaded binary before overwriting the installed one.
if ! "./${EXE}" --version >/dev/null 2>&1; then
  echo "tuxtime: downloaded binary failed to run (${EXE} --version errored)" >&2
  exit 1
fi

mkdir -p "$DEST"
install -m 755 "$EXE" "$DEST/${BIN}"
echo "tuxtime: installed ${VERSION} to ${DEST}/${BIN}"

case ":$PATH:" in
  *":$DEST:"*) ;;
  *)
    echo "tuxtime: note — ${DEST} is not on your PATH; add it:" >&2
    printf '  export PATH="%s:$PATH"\n' "$DEST" >&2
    ;;
esac
