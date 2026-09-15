#!/usr/bin/env bash
# Re-derive the manufacturer-to-country table that
# database/migrations/029_wikidata_country_of_origin.sql carries.
#
# Emits TSV on stdout: family_slug, country, agreeing, sampled, variants.
# It writes nothing and touches no database beyond reading the family list, so
# re-running it is how you check whether 029's thirteen rows still hold or a
# later migration is owed.
#
# The rule, unchanged from the migration: the dominant country counts only when
# it covers at least 90% of at least 10 of a manufacturer's Wikidata aircraft.
# A manufacturer resolved to the wrong entity returns no aircraft and drops out
# on its own.
set -euo pipefail
cd "$(dirname "$0")/.."

UA="aircraft-api-r/0.1 (+https://github.com/rsstdd/aircraft-api-r)"
WORK=$(mktemp -d); trap 'rm -rf "$WORK"' EXIT

docker compose exec -T postgres psql -X -At -U "${POSTGRES_USER:-aircraft}" \
  -d "${POSTGRES_DB:-aircraft}" -c "
  SELECT f.slug||E'\t'||f.name||E'\t'||count(v.id)
    FROM aircraft_core.families f
    JOIN aircraft_core.models m ON m.family_id = f.id
    JOIN aircraft_core.variants v ON v.model_id = m.id
   GROUP BY f.slug, f.name ORDER BY count(v.id) DESC" > "$WORK/families.tsv"

# wbsearchentities' first hit is right for most manufacturers and wrong for
# names that are also ordinary words -- "Piper" resolves to a person. Where the
# bare name yields no aircraft, "<name> Aircraft" is tried before giving up.
resolve() {
  timeout 20 curl -sS -A "$UA" -G "https://www.wikidata.org/w/api.php" \
    --data-urlencode "action=wbsearchentities" --data-urlencode "format=json" \
    --data-urlencode "language=en" --data-urlencode "limit=1" \
    --data-urlencode "search=$1" 2>/dev/null \
  | python3 -c "import sys,json;d=json.load(sys.stdin);print(d['search'][0]['id'] if d.get('search') else '')"
}

countries() {
  timeout 120 curl -sS -A "$UA" -H 'Accept: application/sparql-results+json' \
    -G "https://query.wikidata.org/sparql" --data-urlencode "query=
      SELECT ?iso (COUNT(?ac) AS ?n) WHERE {
        ?ac wdt:P176 wd:$1 . ?ac wdt:P495 ?c . ?c wdt:P298 ?iso
      } GROUP BY ?iso" 2>/dev/null \
  | python3 -c "
import sys, json
try: d = json.load(sys.stdin)
except Exception: sys.exit(0)
for b in d.get('results', {}).get('bindings', []):
    print(b['iso']['value'], b['n']['value'], sep='\t')
"
}

while IFS=$'\t' read -r slug name variants; do
  rows=""
  for term in "$name" "$name Aircraft"; do
    qid=$(resolve "$term" || true)
    [ -n "$qid" ] || continue
    rows=$(countries "$qid" || true)
    [ -n "$rows" ] && break
    sleep 1
  done
  [ -n "$rows" ] || { sleep 1; continue; }
  printf '%s\n' "$rows" | python3 -c "
import sys
counts = [l.split('\t') for l in sys.stdin.read().splitlines() if l.strip()]
counts = [(iso, int(n)) for iso, n in counts]
total = sum(n for _, n in counts)
iso, top = max(counts, key=lambda kv: kv[1])
if total >= 10 and top / total >= 0.90:
    print('$slug', iso, top, total, '$variants', sep='\t')
"
  sleep 2
done < "$WORK/families.tsv"
