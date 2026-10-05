#!/usr/bin/env bash
# Re-derive the model-to-first-flight table that
# database/migrations/030_wikidata_model_first_flight.sql carries.
#
# Emits TSV on stdout: model_slug, first_flight_year. Reads the catalogue and
# Wikidata, writes nothing.
#
# Two rules, both of which the migration re-checks against the database rather
# than trusting this script:
#
#   1. Exact designation match only. Wikidata's "Cessna 172 Skyhawk" first flew
#      in 1955 and this catalogue's "172S Skyhawk SP" in 1998, so any prefix or
#      fuzzy match writes the first onto the second and is wrong by decades.
#   2. A first flight may not postdate the production start the catalogue
#      records. This is not decoration: it rejects the 737-100 and 737-600.
set -euo pipefail
cd "$(dirname "$0")/.."

UA="aircraft-api-r/0.1 (+https://github.com/rsstdd/aircraft-api-r)"
WORK=$(mktemp -d); trap 'rm -rf "$WORK"' EXIT
PSQL=(docker compose exec -T postgres psql -X -At -U "${POSTGRES_USER:-aircraft}" -d "${POSTGRES_DB:-aircraft}")

# Only manufacturers migration 029 established a country for: those are the ones
# whose Wikidata entity is known to resolve to the right company.
"${PSQL[@]}" -c "
  SELECT f.name||E'\t'||m.name||E'\t'||m.slug||E'\t'||coalesce(min(v.production_start_year)::text,'')
    FROM aircraft_core.models m
    JOIN aircraft_core.families f ON f.id = m.family_id
    JOIN aircraft_core.variants v ON v.model_id = m.id
   WHERE f.country_of_origin_code IS NOT NULL
   GROUP BY f.name, m.name, m.slug" > "$WORK/models.tsv"

cut -f1 "$WORK/models.tsv" | sort -u | while read -r maker; do
  qid=$(timeout 20 curl -sS -A "$UA" -G "https://www.wikidata.org/w/api.php" \
      --data-urlencode "action=wbsearchentities" --data-urlencode "format=json" \
      --data-urlencode "language=en" --data-urlencode "limit=1" \
      --data-urlencode "search=$maker Aircraft" 2>/dev/null \
    | python3 -c "import sys,json;d=json.load(sys.stdin);print(d['search'][0]['id'] if d.get('search') else '')")
  [ -n "$qid" ] || continue
  timeout 120 curl -sS -A "$UA" -H 'Accept: application/sparql-results+json' \
    -G "https://query.wikidata.org/sparql" --data-urlencode "query=
      SELECT ?acLabel ?ff WHERE {
        ?ac wdt:P176 wd:$qid . ?ac wdt:P606 ?ff .
        SERVICE wikibase:label { bd:serviceParam wikibase:language \"en\" }
      }" 2>/dev/null \
  | python3 -c "
import sys, json
try: d = json.load(sys.stdin)
except Exception: sys.exit(0)
for b in d.get('results', {}).get('bindings', []):
    label = b['acLabel']['value']
    if not label.startswith('Q'):
        print('$maker', label, b['ff']['value'][:4], sep='\t')
" >> "$WORK/wikidata.tsv" || true
  sleep 2
done

python3 - "$WORK/models.tsv" "$WORK/wikidata.tsv" <<'PY'
import collections, re, sys
models_path, wikidata_path = sys.argv[1], sys.argv[2]

def norm(maker, name):
    name = re.sub(r'\s*\(\d{4}\s*-\s*(?:\d{4}|present)\)\s*$', '', name.strip())
    low, maker_low = name.casefold(), maker.casefold()
    if low.startswith(maker_low + ' '):
        name = name[len(maker) + 1:]
    return re.sub(r'\s+', ' ', name).strip().casefold()

years = collections.defaultdict(lambda: collections.defaultdict(set))
try:
    for line in open(wikidata_path):
        maker, label, year = line.rstrip('\n').split('\t')
        years[maker][norm(maker, label)].add(int(year))
except FileNotFoundError:
    sys.exit(0)

for line in open(models_path):
    maker, name, slug, start = line.rstrip('\n').split('\t')
    found = years[maker].get(norm(maker, name))
    if not found or len(found) != 1:
        continue                                  # absent or ambiguous
    year = next(iter(found))
    if start and year > int(start):
        continue                                  # cannot fly after it is sold
    print(slug, year, sep='\t')
PY
