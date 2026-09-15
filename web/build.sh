#!/usr/bin/env bash
#
# Assemble the playground: the VM compiled to WebAssembly, the static page that
# hosts it, and the repository's examples with a manifest for the page's
# dropdown. The result is a directory of files that any static host can serve --
# GitHub Pages does, and so does `python3 -m http.server`.
#
# Usage: web/build.sh [output directory]   (default: web/dist)

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(dirname "$here")"
dist="${1:-$here/dist}"

echo "==> compiling the VM for wasm32-unknown-unknown"
(cd "$here/wasm" && cargo build --release --target wasm32-unknown-unknown)

echo "==> assembling $dist"
rm -rf "$dist"
mkdir -p "$dist/examples"

cp "$here"/site/index.html "$here"/site/app.css "$here"/site/app.js \
   "$here"/site/worker.js "$dist/"
cp "$here/wasm/target/wasm32-unknown-unknown/release/raft_wasm.wasm" "$dist/raft.wasm"
cp "$root"/examples/*.raft "$dist/examples/"

# GitHub Pages serves this repository's site from a Jekyll build unless told
# otherwise, and Jekyll drops files it does not recognise.
touch "$dist/.nojekyll"

echo "==> writing the example manifest"
manifest="$dist/examples/index.json"
{
  printf '['
  separator=''
  for path in "$root"/examples/*.raft; do
    file="$(basename "$path")"
    name="${file%.raft}"

    # An example's first comment line is its description; without one the file
    # name has to speak for itself.
    summary="$(sed -n 's/^#[[:space:]]*//p' "$path" | head -n 1 | cut -c1-72)"
    if [ -n "$summary" ]; then
      title="$name — $summary"
    else
      title="$name"
    fi

    escaped="$(printf '%s' "$title" | sed 's/\\/\\\\/g; s/"/\\"/g')"
    printf '%s{"file":"%s","name":"%s","title":"%s"}' \
      "$separator" "$file" "$name" "$escaped"
    separator=','
  done
  printf ']\n'
} > "$manifest"

count="$(ls -1 "$dist"/examples/*.raft | wc -l | tr -d ' ')"
size="$(du -h "$dist/raft.wasm" | cut -f1)"

echo
echo "built $dist"
echo "  raft.wasm   $size"
echo "  examples    $count"
echo
echo "preview it with:"
echo "  python3 -m http.server --directory $dist 8080"
