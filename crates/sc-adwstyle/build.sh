#!/bin/sh
# Builds sc-adwstyle (needs libadwaita's development files) into the folder
# given, beside the app's programs, where the app looks for it. CC and
# PKG_CONFIG name another compiler's, to build for another processor.
set -eu
OUT=${1:-target/debug}
mkdir -p "$OUT"
${CC:-cc} -O2 -std=c11 "$(dirname "$0")/sc-adwstyle.c" -o "$OUT/sc-adwstyle" $(${PKG_CONFIG:-pkg-config} --cflags --libs libadwaita-1)
echo "$OUT/sc-adwstyle"
