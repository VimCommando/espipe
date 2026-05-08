#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(grep -o '^version = ".*"' "${ROOT_DIR}/Cargo.toml" | sed -E 's/^version = "(.*)"/\1/')"
REGISTRY="${ESPIPE_REGISTRY:-docker.io/vimcommando}"
IMAGE="${REGISTRY%/}/espipe"

docker buildx build \
    --file "${ROOT_DIR}/docker/Dockerfile" \
    --platform linux/amd64,linux/arm64 \
    --tag "${IMAGE}:latest" \
    --tag "${IMAGE}:${VERSION}" \
    --push \
    "${ROOT_DIR}"
