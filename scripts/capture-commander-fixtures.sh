#!/usr/bin/env bash
# Capture EDHREC's first 1,000 cards and selected land metadata in a new directory.
# This maintenance command uses the network. Existing captures stay unchanged.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: capture-commander-fixtures.sh LAND_GROUPS_JSON NEW_CAPTURE_DIRECTORY" >&2
  echo "       capture-commander-fixtures.sh --behold NEW_CAPTURE_DIRECTORY" >&2
  exit 2
fi

root=$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)
cd "$root"
lands=$1
destination=$2
case "$destination" in
  /*) ;;
  *) destination=./$destination ;;
esac
mode=commander
if [[ "$lands" == --behold ]]; then mode=behold; fi
edhrec_base=https://json.edhrec.com/pages
scryfall_endpoint=https://api.scryfall.com/cards/collection

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
if [[ "$mode" == commander ]]; then
  jq -e '
  .schema_version == 1 and (.groups | type == "array" and length > 0) and
  ([.groups[].id] | length == (unique | length)) and
  all(.groups[]; (.id | type == "string" and length > 0) and
    (.names | type == "array" and length == (unique | length)) and
    (.names | length) == .expected_count and
    all(.names[]; type == "string" and length > 0))
  ' "$lands" >/dev/null
fi
mkdir -p -- "$(dirname "$destination")"
mkdir -- "$destination"
complete=false

# Remove only files in the exclusively created capture on failure.
cleanup() {
  if [[ "$complete" == false ]]; then
    find "$destination" -maxdepth 1 -type f -exec rm -f -- {} +
    rmdir -- "$destination"
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
set -o noclobber

# Hash original file bytes rather than a parsed or formatted representation.
byte_hash() {
  local result
  result=$(shasum -a 256 "$1") || return
  printf '%s' "${result%% *}"
}

# Keep requests at least 150 ms apart, including EDHREC page reads.
fetch() {
  sleep 0.15
  curl --fail-with-body --silent --show-error \
    --connect-timeout 30 --max-time 120 \
    --user-agent "seizethemana-fixture-maintenance/0.1" \
    --header "Accept: application/json" "$@"
}

# Keep all Behold query results in a separate source-only supplement.
capture_behold() {
  local started_at finished_at next page=0 file url source_hash request_hash response_hash
  local selection_hash cards_hash count
  started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  next='https://api.scryfall.com/cards/search?q=keyword%3Abehold%20legal%3Acommander&unique=cards&order=name'
  printf '' >"$destination/source_records.ndjson"
  while [[ -n "$next" ]]; do
    [[ "$next" == https://api.scryfall.com/cards/search\?* ]]
    printf -v file 'behold_search_%02d.json' "$page"
    url=$next
    fetch --output "$destination/$file" "$url"
    jq -e '.object == "list" and (.total_cards | type == "number") and
      (.has_more | type == "boolean") and (.data | type == "array" and length > 0)
    ' "$destination/$file" >/dev/null
    source_hash=$(byte_hash "$destination/$file")
    jq -cn --arg path "$file" --arg url "$url" \
      --arg captured_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
      --arg raw_bytes_sha256 "$source_hash" \
      '{source:"Scryfall",method:"GET",path:$path,url:$url,captured_at:$captured_at,
        raw_bytes_sha256:$raw_bytes_sha256}' >>"$destination/source_records.ndjson"
    next=$(jq -r 'if .has_more then .next_page else "" end' "$destination/$file")
    page=$((page + 1))
  done
  jq -s '{query:"keyword:behold legal:commander",groups:[
      {id:"supplemental_utility",expected_count:1,names:["Elven Passage"]},
      {id:"behold",expected_count:(.[0].total_cards),names:[.[].data[].name]}],
    cards:[.[].data[] | {name,id,oracle_id}]}
  ' "$destination"/behold_search_*.json >"$destination/selection.json"
  jq -e '(.cards | length) == .groups[1].expected_count and
    ([.cards[].name] | length == (unique | length)) and
    ([.cards[].oracle_id] | length == (unique | length)) and
    any(.cards[]; .name == "Elven Passage" and .oracle_id == "97a2cd39-6b54-496b-b3ac-dab9dfed7edc")
  ' "$destination/selection.json" >/dev/null
  count=$(jq '.cards | length' "$destination/selection.json")
  local batch request response batches=$(((count + 74) / 75))
  for ((batch = 0; batch < batches; batch++)); do
    printf -v request 'scryfall_request_%02d.json' "$batch"
    printf -v response 'scryfall_response_%02d.json' "$batch"
    jq --argjson start "$((batch * 75))" \
      '{identifiers:(.cards[$start:($start + 75)] | map({id}))}' \
      "$destination/selection.json" >"$destination/$request"
    fetch --header "Content-Type: application/json" --data-binary "@$destination/$request" \
      --output "$destination/$response" "$scryfall_endpoint"
    jq -e --slurpfile wanted "$destination/$request" '
      . as $response | .object == "list" and .not_found == [] and
      (.data | length) == ($wanted[0].identifiers | length) and
      all($wanted[0].identifiers[]; .id as $id | [$response.data[] | select(.id == $id)] | length == 1)
    ' "$destination/$response" >/dev/null
    request_hash=$(byte_hash "$destination/$request")
    response_hash=$(byte_hash "$destination/$response")
    jq -cn --arg request_path "$request" --arg path "$response" \
      --arg url "$scryfall_endpoint" --arg captured_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
      --arg request_sha256 "$request_hash" --arg raw_bytes_sha256 "$response_hash" \
      '{source:"Scryfall",method:"POST",url:$url,path:$path,request_path:$request_path,
        captured_at:$captured_at,request_sha256:$request_sha256,raw_bytes_sha256:$raw_bytes_sha256}' \
      >>"$destination/source_records.ndjson"
  done
  write_card_metadata
  jq -e --slurpfile selected "$destination/selection.json" '
    . as $cards | length == ($selected[0].cards | length) and
    all($selected[0].cards[]; . as $wanted |
      [$cards[] | select(.id == $wanted.id and .oracle_id == $wanted.oracle_id and .name == $wanted.name)] | length == 1)
  ' "$destination/cards.json" >/dev/null
  selection_hash=$(byte_hash "$destination/selection.json")
  cards_hash=$(byte_hash "$destination/cards.json")
  finished_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  jq -n --arg started_at "$started_at" --arg finished_at "$finished_at" \
    --arg selection_sha256 "$selection_hash" --arg cards_sha256 "$cards_hash" \
    --argjson count "$count" --slurpfile sources "$destination/source_records.ndjson" '
    {schema_version:1,started_at:$started_at,finished_at:$finished_at,
      status:"source_capture_only_not_runtime_support",query:"keyword:behold legal:commander",
      card_count:$count,sources:$sources,files:[
        {path:"selection.json",raw_bytes_sha256:$selection_sha256},
        {path:"cards.json",raw_bytes_sha256:$cards_sha256}]}
  ' >"$destination/capture.json"
  rm -- "$destination/source_records.ndjson"
}

# Preserve simulator input metadata while keeping full API objects in raw source files.
write_card_metadata() {
  jq -s '[.[].data[] | {id,oracle_id,name,mana_cost,cmc,type_line,colors,color_identity,
    keywords,power,toughness,loyalty,oracle_text,rarity,edhrec_rank,penny_rank,reserved,
    legalities,set,collector_number,released_at,game_changer,layout,card_faces}]
  ' "$destination"/scryfall_response_*.json >"$destination/cards.json"
}

if [[ "$mode" == behold ]]; then
  capture_behold
  complete=true
  jq '{started_at,finished_at,card_count,status}' "$destination/capture.json"
  exit 0
fi

started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat "$lands" >"$destination/land_groups.json"
printf '' >"$destination/source_records.ndjson"
next=top/year.json
for page in {0..9}; do
  expected=top/year.json
  if [[ "$page" -gt 0 ]]; then expected="top/year-past2years-$page.json"; fi
  if [[ "$next" != "$expected" ]]; then
    echo "Unexpected EDHREC continuation: $next (expected $expected)" >&2
    exit 1
  fi
  printf -v file 'edhrec_page_%02d.json' "$page"
  url=$edhrec_base/$next
  fetch --output "$destination/$file" "$url"
  if [[ "$page" == 0 ]]; then
    jq -e '.header == "Top Cards (Past 2 Years)" and
      .container.json_dict.cardlists[0].header == "Past 2 Years" and
      .container.json_dict.cardlists[0].tag == "past2years"' "$destination/$file" >/dev/null
  fi
  jq -e '(.container.json_dict.cardlists[0] // .) |
    (.cardviews | type == "array" and length == 100) and
    all(.cardviews[]; (.name | type == "string" and length > 0) and
      (.id | type == "string" and length == 36) and
      (.num_decks | type == "number") and .potential_decks >= .num_decks)
  ' "$destination/$file" >/dev/null
  next=$(jq -r '(.container.json_dict.cardlists[0] // .).more' "$destination/$file")
  source_hash=$(byte_hash "$destination/$file")
  jq -cn --arg path "$file" --arg url "$url" \
    --arg captured_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg raw_bytes_sha256 "$source_hash" \
    --argjson page "$page" \
    '{source:"EDHREC",method:"GET",path:$path,url:$url,captured_at:$captured_at,
      raw_bytes_sha256:$raw_bytes_sha256,page_index:$page}' >>"$destination/source_records.ndjson"
done
[[ "$next" == top/year-past2years-10.json ]]
jq -s '[.[] | (.container.json_dict.cardlists[0] // .).cardviews[]] |
  to_entries | map(.value + {source_position:(.key + 1),page_index:(.key / 100 | floor)})
' "$destination"/edhrec_page_*.json >"$destination/popularity.json"
jq -e 'length == 1000 and ([.[].name] | length == (unique | length)) and
  ([.[].id] | length == (unique | length))' "$destination/popularity.json" >/dev/null

# EDHREC IDs identify printings. Keep their exact name-to-print mapping for review.
jq -n --slurpfile popular "$destination/popularity.json" \
  --slurpfile lands "$destination/land_groups.json" '
  ($popular[0] | map({selection_name:.name,edhrec_id:.id,identifier:{id:.id}})) +
  ([$lands[0].groups[].names[]] | unique | map(. as $name |
    select(all($popular[0][]; .name != $name)) |
    {selection_name:$name,edhrec_id:null,identifier:{name:($name | split(" // ")[0])}}))
' >"$destination/selection.json"
count=$(jq 'length' "$destination/selection.json")
batches=$(((count + 74) / 75))
for ((batch = 0; batch < batches; batch++)); do
  printf -v request 'scryfall_request_%02d.json' "$batch"
  printf -v response 'scryfall_response_%02d.json' "$batch"
  jq --argjson start "$((batch * 75))" \
    '{identifiers:.[ $start : ($start + 75) ] | map(.identifier)}' \
    "$destination/selection.json" >"$destination/$request"
  fetch --header "Content-Type: application/json" \
    --data-binary "@$destination/$request" --output "$destination/$response" "$scryfall_endpoint"
  if ! jq -e --slurpfile requested "$destination/$request" '
    . as $response | .object == "list" and .not_found == [] and
    (.data | type == "array" and length == ($requested[0].identifiers | length)) and
    all($requested[0].identifiers[]; . as $wanted |
      [$response.data[] | select(if $wanted.id then .id == $wanted.id else
        .name == $wanted.name or .card_faces[0].name == $wanted.name end)] | length == 1) and
    all(.data[]; .object == "card" and (.oracle_id | type == "string" and length == 36))
  ' "$destination/$response" >/dev/null; then
    echo "Scryfall response does not match $request" >&2
    jq '{object,not_found,returned:(.data | length)}' "$destination/$response" >&2
    exit 1
  fi
  request_hash=$(byte_hash "$destination/$request")
  response_hash=$(byte_hash "$destination/$response")
  jq -cn --arg request_path "$request" --arg path "$response" \
    --arg url "$scryfall_endpoint" --arg captured_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg request_sha256 "$request_hash" \
    --arg raw_bytes_sha256 "$response_hash" \
    '{source:"Scryfall",method:"POST",url:$url,path:$path,request_path:$request_path,
      captured_at:$captured_at,request_sha256:$request_sha256,raw_bytes_sha256:$raw_bytes_sha256}' \
    >>"$destination/source_records.ndjson"
done
write_card_metadata
jq -e --slurpfile selection "$destination/selection.json" '
  . as $cards | length == ($selection[0] | length) and
  ([.[].oracle_id] | length == (unique | length)) and
  ([.[].name] | length == (unique | length)) and
  all($selection[0][]; . as $selected |
    [$cards[] | select(if $selected.edhrec_id then .id == $selected.edhrec_id else .name == $selected.selection_name end) |
      select(.name == $selected.selection_name or .card_faces[0].name == $selected.selection_name)] | length == 1)
' "$destination/cards.json" >/dev/null
finished_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
selection_hash=$(byte_hash "$destination/selection.json")
popularity_hash=$(byte_hash "$destination/popularity.json")
cards_hash=$(byte_hash "$destination/cards.json")
land_groups_hash=$(byte_hash "$destination/land_groups.json")
jq -n --arg started_at "$started_at" --arg finished_at "$finished_at" \
  --slurpfile sources "$destination/source_records.ndjson" \
  --slurpfile lands "$destination/land_groups.json" \
  --arg selection_sha256 "$selection_hash" \
  --arg popularity_sha256 "$popularity_hash" \
  --arg cards_sha256 "$cards_hash" \
  --arg land_groups_sha256 "$land_groups_hash" \
  --argjson count "$count" '
  {schema_version:1,started_at:$started_at,finished_at:$finished_at,
   status:"source_capture_only_not_runtime_support",reporting_window:"Past 2 Years",
   window_start:null,window_end:null,popularity_count:1000,
   land_count:([$lands[0].groups[].names[]] | unique | length),card_count:$count,
   selection_note:"EDHREC source order, not Scryfall edhrec_rank. The first page plus nine continuation pages form one capture. The reporting window has no exact published bounds. Representative printings may be future reprints of released cards.",
   sources:$sources,files:[
     {path:"selection.json",raw_bytes_sha256:$selection_sha256},
     {path:"popularity.json",raw_bytes_sha256:$popularity_sha256},
     {path:"cards.json",raw_bytes_sha256:$cards_sha256},
     {path:"land_groups.json",raw_bytes_sha256:$land_groups_sha256}]}
' >"$destination/capture.json"
rm -- "$destination/source_records.ndjson"
complete=true
jq '{started_at,finished_at,popularity_count,land_count,card_count,status}' "$destination/capture.json"
