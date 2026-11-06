#!/bin/bash
# Throwaway (tv58 step 0). run.sh <sdk.mjs path> <out dir> <life seconds> <sample count>
set -u
here=$(cd "$(dirname "$0")" && pwd); sdk=$1; out=$2; life=$3; n=$4
mkdir -p "$out"
for v in $(env | sed -n 's/^\(CLAUDE[A-Za-z0-9_]*\)=.*/\1/p'); do unset "$v"; done
cp -R "$here/vitest-project" "$out/vitest-project"; (cd "$out/vitest-project" && npm install --no-audit --no-fund >/dev/null 2>&1; echo "vitest install: $?")
node "$here/fake-api.mjs" "$out/port" & api=$!
while [ ! -s "$out/port" ]; do sleep 0.2; done
export ANTHROPIC_BASE_URL="http://127.0.0.1:$(cat "$out/port")" ANTHROPIC_API_KEY=probe-not-a-key CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 DISABLE_TELEMETRY=1 DISABLE_AUTOUPDATER=1 CLAUDE_CONFIG_DIR="$out/claude-config" PROBE_MODEL=claude-haiku-4-5
mkdir -p "$CLAUDE_CONFIG_DIR"
run() { name=$1; prompt=$2; mcp=${3:-}; d="$out/$name"; mkdir -p "$d"; [ "$name" = vitest ] && d2="$out/vitest-project" ; perl "$here/spawner.pl" "$d" "$life" node "$here/driver.mjs" "$sdk" "$d" "$prompt" $mcp & }
poll='while :; do sleep 60; gh --version >/dev/null; done; : TV58MARK-poll'
run sleep 'RUN::{"input":{"command":"sleep 600 # TV58MARK-sleep","timeout":600000,"description":"long"}}'
run vitest "RUN::{\"input\":{\"command\":\"cd $out/vitest-project && npx vitest run # TV58MARK-vitest\",\"timeout\":600000,\"description\":\"tests\"}}"
run poll "RUN::{\"input\":{\"command\":\"$poll\",\"timeout\":600000,\"description\":\"poll\"}}"
run background "RUN::{\"input\":{\"command\":\"while :; do sleep 60; date; done # TV58MARK-background\",\"run_in_background\":true,\"description\":\"bg\"}}"
run mcp 'RUN::{"input":{"command":"sleep 600 # TV58MARK-mcp","timeout":600000,"description":"long"}}' "$here/mcp-burner.mjs"
node "$here/sampler.mjs" "$out/samples.jsonl" 60 "$n" &
sampler=$!
wait $sampler
sleep $((life > (60 * n) ? life - 60 * n + 60 : 60))
pkill -f TV58MARK 2>/dev/null; kill $api 2>/dev/null
node "$here/analyze.mjs" "$out" sleep vitest poll background mcp | tee "$out/analysis.txt"
