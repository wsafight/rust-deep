#!/usr/bin/env bash
# 在 CI 中安装并校验固定版本的 mdBook。
set -euo pipefail

version="${MDBOOK_VERSION:-v0.5.2}"
install_dir="${MDBOOK_INSTALL_DIR:?请设置 MDBOOK_INSTALL_DIR}"
platform="$(uname -s)-$(uname -m)"

case "$platform" in
  Linux-x86_64)
    target="x86_64-unknown-linux-gnu"
    sha256="084e4342ba564db270108763e404a7d1f309d932651a22484e93c0dc1a071f6d"
    ;;
  Darwin-arm64)
    target="aarch64-apple-darwin"
    sha256="da2f55653e96e3f6e1c53e2e13e91cc0cfbce8ab971c2e0de792c0f1f8d24222"
    ;;
  Darwin-x86_64)
    target="x86_64-apple-darwin"
    sha256="17cc64478ec279a73881420e850bd8f9d460552e56b50159ff465bc97eb90d6c"
    ;;
  *)
    echo "不支持的 mdBook 预编译平台：$platform" >&2
    exit 1
    ;;
esac

archive="mdbook-${version}-${target}.tar.gz"
url="https://github.com/rust-lang/mdBook/releases/download/${version}/${archive}"
tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/rust-deep-mdbook.XXXXXX") || exit 1
trap 'rm -rf -- "$tmpdir"' EXIT

curl -fsSL "$url" -o "$tmpdir/$archive"
printf '%s  %s\n' "$sha256" "$tmpdir/$archive" | shasum -a 256 -c -
mkdir -p "$install_dir"
tar -xzf "$tmpdir/$archive" -C "$install_dir"
"$install_dir/mdbook" --version
