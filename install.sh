#!/usr/bin/env bash
set -euo pipefail

REPOSITORY="revolution-uz/worko-ai-usage"
INSTALL_DIR="${HOME}/.local/bin"
TARGET="${INSTALL_DIR}/worko-ai-usage"
OS="$(uname -s)"
ARCH="$(uname -m)"

case "${OS}" in
  Darwin) PLATFORM="apple-darwin" ;;
  Linux) PLATFORM="unknown-linux-gnu" ;;
  *) echo "Unsupported OS: ${OS}. Windows users should run install.ps1." >&2; exit 1 ;;
esac

case "${ARCH}" in
  x86_64|amd64) CPU="x86_64" ;;
  arm64|aarch64) CPU="aarch64" ;;
  *) echo "Unsupported architecture: ${ARCH}" >&2; exit 1 ;;
esac

ASSET="worko-ai-usage-${CPU}-${PLATFORM}.tar.gz"
URL="https://github.com/${REPOSITORY}/releases/latest/download/${ASSET}"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

echo "Downloading ${ASSET}..."
curl --fail --location --silent --show-error "${URL}" --output "${TMP_DIR}/${ASSET}"
curl --fail --location --silent --show-error \
  "https://github.com/${REPOSITORY}/releases/latest/download/SHA256SUMS" \
  --output "${TMP_DIR}/SHA256SUMS"
EXPECTED="$(awk -v asset="${ASSET}" '$2 == asset { print $1 }' "${TMP_DIR}/SHA256SUMS")"
if [[ -z "${EXPECTED}" ]]; then
  echo "No checksum published for ${ASSET}." >&2
  exit 1
fi
if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL="$(sha256sum "${TMP_DIR}/${ASSET}" | awk '{ print $1 }')"
else
  ACTUAL="$(shasum -a 256 "${TMP_DIR}/${ASSET}" | awk '{ print $1 }')"
fi
if [[ "${ACTUAL}" != "${EXPECTED}" ]]; then
  echo "Checksum verification failed for ${ASSET}." >&2
  exit 1
fi
echo "SHA-256 checksum verified."
tar -xzf "${TMP_DIR}/${ASSET}" -C "${TMP_DIR}"
mkdir -p "${INSTALL_DIR}"
install -m 0755 "${TMP_DIR}/worko-ai-usage" "${TARGET}"

if [[ ":${PATH}:" != *":${INSTALL_DIR}:"* ]]; then
  echo "Add this directory to PATH: ${INSTALL_DIR}"
fi

if [[ "${OS}" == "Darwin" ]]; then
  AGENT="${HOME}/Library/LaunchAgents/uz.worko.ai-usage.plist"
  mkdir -p "$(dirname "${AGENT}")"
  sed "s|__EXECUTABLE__|${TARGET}|g" "${TMP_DIR}/uz.worko.ai-usage.plist" > "${AGENT}"
  launchctl bootout "gui/$(id -u)" "${AGENT}" 2>/dev/null || true
  launchctl bootstrap "gui/$(id -u)" "${AGENT}"
elif command -v systemctl >/dev/null 2>&1; then
  SERVICE_DIR="${HOME}/.config/systemd/user"
  mkdir -p "${SERVICE_DIR}"
  sed "s|__EXECUTABLE__|${TARGET}|g" "${TMP_DIR}/worko-ai-usage.service" > "${SERVICE_DIR}/worko-ai-usage.service"
  cp "${TMP_DIR}/worko-ai-usage.timer" "${SERVICE_DIR}/worko-ai-usage.timer"
  systemctl --user daemon-reload
  systemctl --user enable --now worko-ai-usage.timer
fi

echo "Installed ${TARGET} and enabled hourly sync."
exec "${TARGET}" login "$@"
