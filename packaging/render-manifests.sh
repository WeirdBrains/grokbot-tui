#!/usr/bin/env bash
# Fill the AUR PKGBUILD and winget manifest templates from real release checksums.
#
# Usage: render-manifests.sh TAG VERSION CHECKSUM_DIR SOURCE_DIR OUT_DIR
#   TAG           release tag, e.g. v0.2.0
#   VERSION       package version, e.g. 0.2.0
#   CHECKSUM_DIR  directory with the release's *.sha256 files (dist's "<hash>  <name>" format)
#   SOURCE_DIR    top-level directory inside the release's source.tar.gz
#   OUT_DIR       output directory (aur/<pkg>/PKGBUILD, winget/manifests/...)
#
# The license comes from Cargo.toml; identity values from packaging/identity.env.
# Fails if a checksum is missing or any @PLACEHOLDER@ is left unfilled.
set -euo pipefail

if [[ $# -ne 5 ]]; then
  sed -n '2,12p' "$0" >&2
  exit 2
fi
tag=$1 version=$2 sums=$3 srcdir=$4 out=$5
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/.." && pwd)

# shellcheck source=SCRIPTDIR/identity.env
source "$here/identity.env"

license=$(cargo metadata --no-deps --format-version 1 --manifest-path "$root/Cargo.toml" |
  jq -r '.packages[] | select(.name == "gb") | .license')
[[ -n $license && $license != null ]] || { echo "no license in Cargo.toml" >&2; exit 1; }

sha() {
  local f="$sums/$1.sha256" h
  [[ -f $f ]] || { echo "missing checksum file: $f" >&2; exit 1; }
  h=$(awk '{print $1; exit}' "$f")
  [[ $h =~ ^[0-9a-fA-F]{64}$ ]] || { echo "bad sha256 in $f" >&2; exit 1; }
  printf '%s' "${h,,}"
}

sha_linux_x86_64=$(sha gb-x86_64-unknown-linux-gnu.tar.xz)
sha_linux_aarch64=$(sha gb-aarch64-unknown-linux-gnu.tar.xz)
sha_source=$(sha source.tar.gz)
sha_msi=$(sha gb-x86_64-pc-windows-msvc.msi)

render() { # template output
  mkdir -p "$(dirname "$2")"
  sed \
    -e "s|@TAG@|$tag|g" \
    -e "s|@VERSION@|$version|g" \
    -e "s|@LICENSE@|$license|g" \
    -e "s|@MAINTAINER@|$MAINTAINER|g" \
    -e "s|@PUBLISHER@|$PUBLISHER|g" \
    -e "s|@WINGET_ID@|$WINGET_ID|g" \
    -e "s|@SOURCE_DIR@|$srcdir|g" \
    -e "s|@SHA256_LINUX_X86_64@|$sha_linux_x86_64|g" \
    -e "s|@SHA256_LINUX_AARCH64@|$sha_linux_aarch64|g" \
    -e "s|@SHA256_SOURCE@|$sha_source|g" \
    -e "s|@SHA256_WINDOWS_MSI@|${sha_msi^^}|g" \
    "$1" > "$2"
  if grep -n '@[A-Z0-9_]*@' "$2"; then
    echo "unfilled placeholder in $2" >&2
    exit 1
  fi
}

for pkg in grokbot-tui-bin grokbot-tui; do
  render "$here/aur/$pkg/PKGBUILD.in" "$out/aur/$pkg/PKGBUILD"
done

# winget-pkgs layout: manifests/<first letter>/<Publisher>/<Package>/<version>/
id_path=${WINGET_ID//.//}
wdir="$out/winget/manifests/$(printf '%s' "${WINGET_ID:0:1}" | tr '[:upper:]' '[:lower:]')/$id_path/$version"
render "$here/winget/version.yaml.in" "$wdir/$WINGET_ID.yaml"
render "$here/winget/installer.yaml.in" "$wdir/$WINGET_ID.installer.yaml"
render "$here/winget/locale.en-US.yaml.in" "$wdir/$WINGET_ID.locale.en-US.yaml"

echo "rendered into $out:"
find "$out" -type f | sort
