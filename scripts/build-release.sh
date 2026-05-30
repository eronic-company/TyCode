#!/bin/bash
# Build TyCode for multiple platforms and create release artifacts

set -e

VERSION=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
echo "Building TyCode v$VERSION..."

# Create output directory
mkdir -p releases

# Supported targets
TARGETS=(
  "x86_64-unknown-linux-gnu"
  "aarch64-unknown-linux-gnu"
  "x86_64-apple-darwin"
  "aarch64-apple-darwin"
  "x86_64-pc-windows-msvc"
)

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

for target in "${TARGETS[@]}"; do
  echo -e "${YELLOW}Building for $target...${NC}"

  # Check if target is installed
  rustup target add "$target" 2>/dev/null || true

  # Build
  cargo build --release --target "$target" 2>&1 | grep -E "(Compiling|Finished|error)" || true

  # Find and copy binary
  if [[ "$target" == *"windows"* ]]; then
    binary="target/$target/release/tycode.exe"
    binary_name="tycode.exe"
  else
    binary="target/$target/release/tycode"
    binary_name="tycode"
  fi

  if [ -f "$binary" ]; then
    # Create archive
    case "$target" in
      *"linux-gnu")
        arch="${target%%-*}"
        archive="releases/tycode-linux-$arch.tar.gz"
        tar -czf "$archive" -C "$(dirname $binary)" "$(basename $binary)"
        ;;
      *"darwin")
        arch="${target%%-*}"
        archive="releases/tycode-macos-$arch.tar.gz"
        tar -czf "$archive" -C "$(dirname $binary)" "$(basename $binary)"
        ;;
      *"windows")
        arch="${target%%-*}"
        archive="releases/tycode-windows-$arch.zip"
        cd "$(dirname $binary)" && zip -q "$OLDPWD/$archive" "$(basename $binary)" && cd "$OLDPWD"
        ;;
    esac

    echo -e "${GREEN}✓ Created $archive${NC}"
  else
    echo -e "${YELLOW}✗ Build failed for $target${NC}"
  fi
done

echo ""
echo -e "${GREEN}Releases ready in ./releases/${NC}"
ls -lh releases/
