#!/usr/bin/env bash
# Build and sign ovim for local development
# Usage: ./build-and-sign.sh [--debug|--release]

set -euo pipefail

PROFILE="${1:---release}"
PROFILE="${PROFILE#--}"  # Strip leading --

echo "Building ovim (profile: $PROFILE)..."
cargo build --profile "$PROFILE"

BINARY="./target/$PROFILE/ovim"

if [[ ! -f "$BINARY" ]]; then
    echo "Error: Binary not found at $BINARY"
    exit 1
fi

echo "Signing binary..."
codesign --sign - --force "$BINARY"

echo "Verifying signature..."
codesign --verify --verbose "$BINARY"

echo "✓ Build and sign complete: $BINARY"
