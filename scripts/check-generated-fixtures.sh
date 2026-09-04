#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd -P)
manifest="$repo_root/fixtures/prismpm-1.1/fixture-manifest.toml"

mapfile -t source_paths < <(sed -n 's/^source_path = "\([^"]*\)"$/\1/p' "$manifest")
mapfile -t source_hashes < <(sed -n 's/^source_sha256 = "\([0-9a-f]*\)"$/\1/p' "$manifest")
mapfile -t generated_paths < <(sed -n 's/^generated_path = "\([^"]*\)"$/\1/p' "$manifest")
mapfile -t generated_hashes < <(sed -n 's/^generated_sha256 = "\([0-9a-f]*\)"$/\1/p' "$manifest")

test "${#source_paths[@]}" -eq 25
test "${#source_hashes[@]}" -eq "${#source_paths[@]}"
test "${#generated_paths[@]}" -eq "${#source_paths[@]}"
test "${#generated_hashes[@]}" -eq "${#source_paths[@]}"

test "$(printf '%s\n' "${source_paths[@]}" | sort -u | wc -l)" -eq "${#source_paths[@]}"
test "$(printf '%s\n' "${generated_paths[@]}" | sort -u | wc -l)" -eq "${#generated_paths[@]}"

mapfile -t actual_generated_paths < <(
  find "$repo_root/lean/PrismPM/Foundation" -type f -name '*.lean' -printf '%P\n' |
    sed 's#^#lean/PrismPM/Foundation/#' |
    sort
)
mapfile -t registered_generated_paths < <(printf '%s\n' "${generated_paths[@]}" | sort)
test "${actual_generated_paths[*]}" = "${registered_generated_paths[*]}"

for index in "${!source_paths[@]}"; do
  printf '%s  %s\n' "${source_hashes[$index]}" "$repo_root/${source_paths[$index]}" | sha256sum -c -
  printf '%s  %s\n' "${generated_hashes[$index]}" "$repo_root/${generated_paths[$index]}" | sha256sum -c -
done

if grep -En '\<(sorry|admit|axiom|opaque)\>' "${generated_paths[@]/#/$repo_root/}"; then
  echo "generated Prism fixture contains a forbidden declaration" >&2
  exit 1
fi

echo "generated-fixture provenance passed"
