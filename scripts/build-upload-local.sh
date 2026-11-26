#!/usr/bin/env bash
# Build the current checkout or fetch a published release, then upload packages.
# Does not run Ansible or restart services.
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
host=rs9tm
destination=/opt/localbox/yoloforgeincoming
published=
if [[ $# -ne 0 ]]; then
    if [[ $# -ne 2 || $1 != --release || ! $2 =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?$ ]]; then
        printf 'Usage: %s [--release vX.Y.Z]\n' "$0" >&2
        exit 2
    fi
    published=$2
fi
builder=${YOLOFORGE_BUILD_IMAGE:-rust:1.98-bookworm}
scratch=${TMPDIR:-/tmp}
if [[ -d /tmp/opencode && -z ${TMPDIR:-} ]]; then
    scratch=/tmp/opencode
fi
build=$(mktemp -d "$scratch/yoloforge-build.XXXXXXXX")
trap 'rm -rf -- "$build"' EXIT
mkdir -p "$build"/{source,cargo,target,packages}

if [[ -n $published ]]; then
    version=$published
    base="https://github.com/farion/yoloforge/releases/download/$version"
    for kind in server runner; do
        name="yoloforge-$kind-$version-linux-x86_64.tar.gz"
        curl -fsSL --retry 2 "$base/$name" -o "$build/packages/$name"
        curl -fsSL --retry 2 "$base/$name.sha256" -o "$build/packages/$name.sha256"
    done
    (cd -- "$build/packages" && sha256sum --check -- *.sha256)
    prefix=release
else
# Include local changes, but not .git, ignored local state, or generated output.
git -C "$root" ls-files -z --cached --others --exclude-standard |
    rsync -a --ignore-missing-args --from0 --files-from=- "$root/" "$build/source/"

docker run --rm --platform linux/amd64 --user "$(id -u):$(id -g)" \
    -e HOME=/cargo -e CARGO_HOME=/cargo -e CARGO_TARGET_DIR=/build -e CARGO_INCREMENTAL=0 \
    -v "$build/source:/src:ro" -v "$build/cargo:/cargo" -v "$build/target:/build" \
    -w /src "$builder" cargo build --locked --release -p factoryd -p factory-runner

version=$(python3 -c 'import sys, tomllib; print("v" + tomllib.load(open(sys.argv[1], "rb"))["workspace"]["package"]["version"])' \
    "$build/source/Cargo.toml")
for kind in server runner; do
    python3 "$build/source/scripts/package-release.py" --kind "$kind" --os linux \
        --arch x86_64 --version "$version" --binaries "$build/target/release" \
        --output "$build/packages"
done
    prefix=local-build
fi

ssh=(ssh -o BatchMode=yes "$host")
stage=$("${ssh[@]}" bash -s -- "$destination" <<'REMOTE'
set -euo pipefail
stage=$(sudo -n mktemp -d "$1/.incoming.XXXXXXXX")
sudo -n chown "$(id -u):$(id -g)" "$stage"
printf '%s\n' "$stage"
REMOTE
)
if [[ ! $stage =~ ^${destination}/\.incoming\.[a-zA-Z0-9]{8}$ ]]; then
    printf 'Unexpected staging directory: %s\n' "$stage" >&2
    exit 1
fi

scp -o BatchMode=yes "$build/packages/"*.tar.gz "$build/packages/"*.tar.gz.sha256 "$host:$stage/"
"${ssh[@]}" bash -s -- "$stage" "$version" "$prefix" <<'REMOTE'
set -euo pipefail
stage=$1
version=$2
cd -- "$stage"
sha256sum --check -- *-linux-x86_64.tar.gz.sha256
cd -- ..
release="$3-$version-${stage##*.}"
chmod 0755 -- "$stage"
sudo -n mv -T -- "$stage" "$release"
printf 'Packages: %s/%s\n' "$PWD" "$release"
REMOTE
