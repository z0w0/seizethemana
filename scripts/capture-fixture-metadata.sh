#!/usr/bin/env bash
# Save an explicit Scryfall lookup without changing existing captures.
# Paths are relative to the repository root. This command uses the network.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: capture-fixture-metadata.sh NAMES_JSON NEW_CAPTURE_DIRECTORY" >&2
  exit 2
fi

root=$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)
cd "$root"
names=$1
destination=$2
endpoint=https://api.scryfall.com/cards/collection

if [[ -e "$destination" || -L "$destination" ]]; then
  echo "Capture destination already exists: $destination" >&2
  exit 1
fi

parent=$(dirname "$destination")
while [[ "$parent" != / && "$parent" != . ]]; do
  if [[ -L "$parent" ]]; then
    echo "Capture parent is a symlink: $parent" >&2
    exit 1
  fi
  parent=$(dirname "$parent")
done

jq -e 'type == "array" and length > 0 and length <= 75 and all(.[]; type == "string" and length > 0)' "$names" >/dev/null
mkdir -p -- "$(dirname "$destination")"
mkdir -- "$destination"
complete=false

# The exclusive mkdir owns these three files. A failed capture can be retried.
cleanup() {
  if [[ "$complete" == false ]]; then
    rm -f -- "$destination/request.json" "$destination/response.json" "$destination/capture.json"
    rmdir -- "$destination"
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

set -o noclobber
jq '{identifiers: map({name: .})}' "$names" >"$destination/request.json"
curl --fail-with-body --silent --show-error \
  --user-agent "seizethemana-fixture-maintenance/0.1" \
  --header "Accept: application/json" \
  --header "Content-Type: application/json" \
  --data-binary "@$destination/request.json" \
  --output "$destination/response.json" \
  "$endpoint"
jq -e '.object == "list" and (.data | type == "array") and (.not_found | type == "array")' "$destination/response.json" >/dev/null

captured_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
request_sha256=$(shasum -a 256 "$destination/request.json")
response_sha256=$(shasum -a 256 "$destination/response.json")
jq --null-input \
  --arg endpoint "$endpoint" \
  --arg captured_at "$captured_at" \
  --arg request_sha256 "${request_sha256%% *}" \
  --arg response_sha256 "${response_sha256%% *}" \
  '{source: "Scryfall", endpoint: $endpoint, method: "POST", operation: "read-only card collection lookup", captured_at: $captured_at, request_sha256: $request_sha256, response_sha256: $response_sha256}' \
  >"$destination/capture.json"
jq '{returned: (.data | length), not_found}' "$destination/response.json"
complete=true
