#!/usr/bin/env bash
# Gagal jika ada commit di rentang BASE..HEAD yang membawa atribusi alat AI,
# baik di pesan commit maupun di identitas author/committer.
# Pemakaian: scripts/check-ai-attribution.sh <base-sha> <head-sha>
set -euo pipefail

base_sha="$1"
head_sha="$2"
msg_pattern='^(co-authored-by:.*(claude|anthropic|openai|chatgpt|codex|copilot|gemini|cursor|codeium|windsurf|devin|aider)|claude-session:|.*generated with .*(claude|copilot|cursor|codex|gemini|aider))'
id_pattern='(claude|anthropic|openai|codex|copilot|cursor|codeium|devin|aider)'

fail=0
for sha in $(git rev-list "$base_sha..$head_sha"); do
    if git log -1 --format=%B "$sha" | grep -Eiq "$msg_pattern"; then
        echo "::error::commit ${sha:0:7} memuat trailer atribusi AI di pesan commit"
        fail=1
    fi
    ident="$(git log -1 --format='%an <%ae>%n%cn <%ce>' "$sha")"
    if grep -Eiq "$id_pattern" <<< "$ident"; then
        echo "::error::commit ${sha:0:7} dibuat dengan identitas alat AI: $(head -1 <<< "$ident")"
        fail=1
    fi
done
exit "$fail"
