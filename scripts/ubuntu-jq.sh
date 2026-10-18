#!/bin/sh
# Run the comparison tests against Ubuntu's jq, the way the parity job does.
# That job runs on ubuntu-latest, which is Ubuntu 24.04, and installs jq with
# apt. The container is linux/amd64, which is that runner's architecture.
set -eu

cd "$(dirname "$0")/.."

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required" >&2
  exit 1
fi

# Many jq processes at once stall when linux/amd64 is emulated.
threads=8
if [ "$(uname -m)" != "x86_64" ]; then
  threads=1
fi

docker run --platform linux/amd64 --rm -i \
  -v "$PWD":/src \
  --tmpfs /src/target \
  -w /src \
  -v tumulus-ubuntu-cargo:/usr/local/cargo \
  -v tumulus-ubuntu-rustup:/usr/local/rustup \
  -v tumulus-ubuntu-target:/tmp/target \
  -e CARGO_HOME=/usr/local/cargo \
  -e RUSTUP_HOME=/usr/local/rustup \
  -e CARGO_TARGET_DIR=/tmp/target \
  -e PATH=/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  -e TUMULUS_TEST_THREADS="$threads" \
  ubuntu:24.04 \
  sh -eu -s <<'EOF'
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y jq curl ca-certificates build-essential pkg-config
if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
fi
jq --version
stdbuf -oL -eL cargo test --locked -- --test-threads="$TUMULUS_TEST_THREADS"
EOF
