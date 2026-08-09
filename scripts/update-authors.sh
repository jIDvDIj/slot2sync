#!/usr/bin/env sh
# Gera o arquivo AUTHORS a partir do git log, ordenado por número de commits.
# Identidades duplicadas são unificadas via .mailmap; bots e trailers de
# ferramentas são filtrados.
#
# Uso: sh scripts/update-authors.sh
set -eu

cd "$(git rev-parse --show-toplevel)"

tmp=$(mktemp)
{
  # Autores diretos (%aN/%aE já aplicam o .mailmap)…
  git log --format="%aN <%aE>"
  # …e coautores declarados em trailers Co-authored-by.
  git log --format="%(trailers:key=Co-authored-by,valueonly)" | sed '/^[[:space:]]*$/d'
} \
  | grep -viE '\[bot\]|noreply@anthropic\.com|actions@github\.com' \
  | sort | uniq -c | sort -rn \
  | sed -E 's/^ *[0-9]+ //' > "$tmp"

{
  echo "# Contribuidores do Slot2Sync, ordenados por número de commits."
  echo "# Gerado por scripts/update-authors.sh — não edite manualmente."
  cat "$tmp"
} > AUTHORS
rm -f "$tmp"

echo "AUTHORS atualizado ($(grep -c '<' AUTHORS) contribuidores)."
