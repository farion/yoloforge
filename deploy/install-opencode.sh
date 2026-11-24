#!/bin/sh
set -eu

version=2.4.1
case "$(dpkg --print-architecture)" in
  amd64) arch=x64; checksum=1584009e779594b9aacbceb6ebbe2fbd15648fcc87b990051c9195dd4086431a3c68a1b9f081cf2abb296f1aee55a8e7204a5bd2df0f78eeb71ffa16e2f246e4 ;;
  arm64) arch=arm64; checksum=a2534a7e802e1da0ed1eb5ab96ac61d913d2a2749f0193c6265aeeeed30d78d4e51f22d2ad36c37bff8727abdd611a7b838c8a2d243c486dbae14fc80bb3e0b7 ;;
  *) echo "Unsupported OpenCode architecture" >&2; exit 1 ;;
esac
work="$(mktemp -d)"
trap 'rm -f "$work/archive.tgz" "$work/opencode"; rmdir "$work"' EXIT
curl -fsSL --retry 3 "https://registry.npmjs.org/@opencode/cli-linux-$arch/-/cli-linux-$arch-$version.tgz" -o "$work/archive.tgz"
printf '%s  %s\n' "$checksum" "$work/archive.tgz" | sha512sum -c -
tar -xOzf "$work/archive.tgz" package/bin/opencode >"$work/opencode"
install -m 0755 "$work/opencode" /usr/local/bin/opencode
test "$(/usr/local/bin/opencode --version)" = "opencode v$version"
