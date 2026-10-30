#!/usr/bin/env bash
# Test release steps without publishing. Requires Bash 3.2+, Cargo, jq,
# standard Unix tools, and tar with xz support.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
for tool in awk bash cargo cat chmod cp grep jq mkdir mktemp rm sed tar; do
  command -v "$tool" >/dev/null || {
    printf 'Missing required tool: %s\n' "$tool" >&2
    exit 1
  }
done
temp=$(mktemp -d "${TMPDIR:-/tmp}/stm-release.XXXXXX")
trap 'rm -rf "$temp"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
workflow="$root/.github/workflows/build.yml"
version=1.2.3
matrix_expression="\${{ matrix.target }}"

# Report a broken release contract.
fail() {
  printf '%s\n' "$*" >&2
  exit 1
}

# Read a named step from the build workflow.
workflow_step() {
  awk -v name="      - name: $1" '
    $0 == name { found = 1; print; next }
    found && /^      - / { exit }
    found { print }
    END { if (!found) exit 1 }
  ' "$workflow" || fail "Missing workflow step: $1"
}

# Extract the shell code that GitHub Actions runs.
shell_script() {
  awk '
    /^        run: \|$/ { found = 1; next }
    found { sub(/^          /, ""); print }
    END { if (!found) exit 1 }
  ' || fail 'Missing literal workflow run block.'
}

# Resolve cargo-binstall placeholders for the current target.
expand_template() {
  local template
  template=$(printf '%s\n' "$1" | sed -E 's/\{[[:space:]]*([^{}[:space:]]+)[[:space:]]*\}/{\1}/g')
  template=${template//\{repo\}/$repository}
  template=${template//\{name\}/$name}
  template=${template//\{version\}/$version}
  template=${template//\{target\}/$target}
  template=${template//\{archive-suffix\}/$suffix}
  template=${template//\{bin\}/stm}
  template=${template//\{binary-ext\}/$binary_ext}
  [[ $template != *'{'* && $template != *'}'* ]] || fail "Unknown binstall placeholder: $template"
  printf '%s\n' "$template"
}

# Run the real Unix package and upload scripts with a local gh stub.
check_unix_target() (
  [[ $package_format == txz ]] || fail "Expected txz format for $target."
  local directory package_script archive_line
  directory="$temp/$target"
  mkdir -p "$directory/target/$target/release"
  printf 'stub binary\n' > "$directory/target/$target/release/stm"
  cp "$root/README.md" "$root/LICENSE" "$directory/"
  cd "$directory"
  export GITHUB_REF_NAME="v$version" GITHUB_REF="refs/tags/v$version"
  export GITHUB_REPOSITORY=test/repo GITHUB_ENV="$directory/github-env"
  export PATH="$temp/bin:$PATH" GH_TOKEN=release-check-only
  unset archive ARCHIVE
  package_script=${unix_script//"$matrix_expression"/$target}
  bash -euo pipefail -c "$package_script"
  archive_line=$(cat "$GITHUB_ENV")
  [[ $archive_line == "archive=$expected" ]] || fail "Shared archive variable mismatch for $target."
  export archive="${archive_line#archive=}"
  tar -tJf "$archive" | grep -Fx -- "$(expand_template "$bin_dir")" >/dev/null || fail "Missing binstall binary in $archive."
  bash -euo pipefail -c "$release_script"
  [[ $(sed -n '4p' upload-args) == "$expected" ]] || fail "Uploaded archive mismatch for $target."
)

cargo metadata --manifest-path "$root/Cargo.toml" --no-deps --format-version 1 \
  --offline --locked > "$temp/metadata.json"
jq -er --arg manifest "$root/Cargo.toml" \
  '.packages[] | select(.manifest_path == $manifest)' "$temp/metadata.json" > "$temp/package.json"
name=$(jq -er '.name' "$temp/package.json")
repository=$(jq -er '.repository' "$temp/package.json")
pkg_url=$(jq -er '.metadata.binstall."pkg-url"' "$temp/package.json")
package_format=$(jq -er '.metadata.binstall."pkg-fmt"' "$temp/package.json")
bin_dir=$(jq -er '.metadata.binstall."bin-dir"' "$temp/package.json")

workflow_step 'Upload workflow artifact' > "$temp/artifact-step"
grep -F "name: \${{ env.archive }}" "$temp/artifact-step" >/dev/null || fail 'Artifact name must use env.archive.'
grep -F "path: \${{ env.archive }}" "$temp/artifact-step" >/dev/null || fail 'Artifact path must use env.archive.'
workflow_step 'Package (unix)' | shell_script > "$temp/unix-script"
unix_script=$(cat "$temp/unix-script")
workflow_step 'Attach archive to GitHub release (tags only)' > "$temp/release-step"
grep -F "if: startsWith(github.ref, 'refs/tags/v')" "$temp/release-step" >/dev/null || fail 'Release uploads must run only for version tags.'
release_script=$(shell_script < "$temp/release-step")

mkdir "$temp/bin"
cat > "$temp/bin/gh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
if [[ $# -ge 2 && $1 == release && $2 == create ]]; then
  exit 1
fi
[[ $# -eq 7 && $1 == release && $2 == upload ]] || {
  printf 'Unexpected gh arguments: %s\n' "$*" >&2
  exit 1
}
[[ $3 == "$GITHUB_REF_NAME" && -f $4 ]] || {
  printf 'Upload tag or archive is invalid.\n' >&2
  exit 1
}
printf '%s\n' "$@" > upload-args
STUB
chmod +x "$temp/bin/gh"

awk '/^          - target: / { print $3 }' "$workflow" > "$temp/targets"
[[ -s $temp/targets ]] || fail 'Release target matrix is empty.'
count=0
while IFS= read -r target; do
  suffix=.tar.xz
  binary_ext=
  if [[ $target == *windows* ]]; then
    suffix=.zip
    binary_ext=.exe
  fi
  expected="$name-$version-$target$suffix"
  [[ $(expand_template "$pkg_url") == *"/v$version/$expected" ]] || fail "Binstall URL mismatch for $target."
  if [[ $target == *windows* ]]; then
    [[ $(jq -er --arg target "$target" '.metadata.binstall.overrides[$target]."pkg-fmt"' "$temp/package.json") == zip ]] \
      || fail "Expected zip format for $target."
  else
    check_unix_target
  fi
  count=$((count + 1))
done < "$temp/targets"
printf 'Checked release metadata for %s targets and tested Unix archives and uploads.\n' "$count"
