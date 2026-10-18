#!/bin/sh
# Install every locale date_locale_matches_jq compares.
# On Linux, locale-gen builds the language locales. C, POSIX, and C.utf8
# are already part of glibc, and glibc accepts the UTF-8 spelling of those
# names too. Apple ships the UTF-8 directories only, so the utf8 names are
# links to them.
set -eu

as_root() {
  if [ "$(id -u)" -eq 0 ]; then
    "$@"
  else
    sudo "$@"
  fi
}

if [ "$(uname)" = "Darwin" ]; then
  for spec in C en_US en_GB de_DE fr_FR ja_JP; do
    src="/usr/share/locale/${spec}.UTF-8"
    dst="/usr/share/locale/${spec}.utf8"
    if [ -e "$dst" ]; then
      continue
    fi
    if [ ! -d "$src" ]; then
      echo "missing locale directory $src" >&2
      exit 1
    fi
    as_root ln -s "${spec}.UTF-8" "$dst"
  done
  exit 0
fi

export DEBIAN_FRONTEND=noninteractive
as_root apt-get update
as_root apt-get install -y locales
as_root locale-gen en_US.UTF-8 en_GB.UTF-8 de_DE.UTF-8 fr_FR.UTF-8 ja_JP.UTF-8
