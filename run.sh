#!/bin/bash
set -e

cd "$(dirname "$0")"

if [ "$1" = "build" ] || [ ! -f target/release/clicontrol ]; then
    echo "Building clicontrol (release)..."
    cargo build --release
fi

exec ./target/release/clicontrol "$@"
