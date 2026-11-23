#!/usr/bin/env bash
# Test harness for .github/scripts/model-lock.sh (NEX-91, RFC 0001 "The model": "Building it" and
# "The model only grows").
#
# The script is CI's model lock step: `pin` names the NoCentralDB version the repository pins,
# `install` puts `ncdb` at that version into a directory, and `check` rebuilds model/model.lock.json
# from model/model.ts and checks the model against the lock on the base branch. Every way `check`
# can answer is run here against a real git repository on disk with the REAL `ncdb` and Node 24,
# because what decides "additive" is ncdb, and a stub would only test the stub. `install` runs
# against a stubbed `cargo`, so the harness compiles nothing.
#
# Needs: bash, git, Node 24 or later and `ncdb` at the pinned version on PATH, and model/node_modules
# (`npm ci` in model/). CI's lint job runs it after it has installed all three.
#
# MUTATION-CHECKED, recorded rather than asserted (the bar `tests/require-green-ci.test.sh` sets).
# Each perturbation below was planted in the script, seen RED here, and reverted:
#
#   M1  no build over the base lock          → the removal and the rename rebuilt from nothing  2 red
#   M2  no byte comparison after the rebuild → model.ts without a rebuild, the hand-edited lock  2 red
#   M3  `cargo install` without `=` and --locked → the empty root, the other version          2 red
#   M4  model/package.json's SDK pin not compared with Cargo.lock → the differing SDK pin      1 red
#
# Run: tests/model-lock.test.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$ROOT/.github/scripts/model-lock.sh"

PASS=0
FAIL=0
ok()  { PASS=$((PASS + 1)); printf '  ok   %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf '  FAIL %s\n' "$1"; }

[ -d "$ROOT/model/node_modules" ] || { echo "model/node_modules is missing: run npm ci in model/ first" >&2; exit 2; }
command -v ncdb >/dev/null || { echo "ncdb is not on PATH" >&2; exit 2; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/nxf-model-lock-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK/home"
mkdir -p "$HOME"
git config --global init.defaultBranch v2
git config --global advice.detachedHead false

PIN="$(sed -n 's/^ *"@nocentraldb\/sdk": *"\([^"]*\)".*/\1/p' "$ROOT/model/package.json")"

# A repository whose first commit is the base branch: Cargo.lock and model/ as they are in this
# checkout, with node_modules linked in. Prints its path.
fixture() {
  local repo
  repo="$(mktemp -d "$WORK/repo.XXXXXX")"
  mkdir -p "$repo/model"
  cp "$ROOT/Cargo.lock" "$repo/"
  cp "$ROOT/model/model.ts" "$ROOT/model/model.lock.json" "$ROOT/model/package.json" \
     "$ROOT/model/package-lock.json" "$repo/model/"
  ln -s "$ROOT/model/node_modules" "$repo/model/node_modules"
  printf 'node_modules\n' > "$repo/.gitignore"
  git -C "$repo" init -q
  git -C "$repo" add -A
  git -C "$repo" commit -qm base
  git -C "$repo" tag base
  printf '%s\n' "$repo"
}

# Adds a field to spaceMember in model.ts, after `ticket`.
add_field() { # <repo> <name>
  sed -i.bak "s|^    ticket: nc.ref(\[\]),|    ticket: nc.ref([]),\n    $2: nc.string(),|" "$1/model/model.ts"
  rm -f "$1/model/model.ts.bak"
}
# Rebuilds the committed lock the way a coder does.
rebuild() { (cd "$1/model" && ncdb model build >/dev/null); }
# Builds the lock from nothing, as a PR would that hid a removal by starting a new lock.
rebuild_fresh() { rm "$1/model/model.lock.json"; rebuild "$1"; }
commit() { git -C "$1" add -A && git -C "$1" commit -qm change; }

RC=0
OUT=""
check() { # <repo> <base-rev>
  RC=0
  OUT="$(cd "$1" && bash "$SCRIPT" check "$2" 2>&1)" || RC=$?
}
expect_pass() { # <case> <repo>
  check "$2" base
  if [ "$RC" = 0 ]; then ok "$1"; else bad "$1 (exit $RC: $OUT)"; fi
}
expect_fail() { # <case> <repo> <text the output names>
  check "$2" base
  if [ "$RC" != 0 ] && grep -qF -- "$3" <<<"$OUT"; then ok "$1"; else bad "$1 (exit $RC, wanted '$3': $OUT)"; fi
}

# ---- check: the rebuild ----------------------------------------------------------------------

echo "check: the committed lock is what ncdb model build writes"

r="$(fixture)"
expect_pass "an unchanged model passes" "$r"
if [ -z "$(git -C "$r" status --porcelain)" ]; then ok "check leaves the working tree as it was"; else bad "check leaves the working tree as it was ($(git -C "$r" status --porcelain))"; fi

r="$(fixture)"; add_field "$r" note; rebuild "$r"; commit "$r"
expect_pass "a new field with the lock rebuilt passes" "$r"

r="$(fixture)"; add_field "$r" note; rebuild "$r"; commit "$r"; add_field "$r" extra; rebuild "$r"; commit "$r"
expect_pass "two rebuilds on one branch pass (model version 3 over a base at 1)" "$r"

r="$(fixture)"; add_field "$r" note; commit "$r"
expect_fail "model.ts edited without rebuilding the lock fails" "$r" "model.lock.json"
if [ -z "$(git -C "$r" status --porcelain)" ]; then ok "a failed check leaves the working tree as it was"; else bad "a failed check leaves the working tree as it was ($(git -C "$r" status --porcelain))"; fi

r="$(fixture)"; printf '\n' >> "$r/model/model.lock.json"; commit "$r"
expect_fail "a lock edited by hand fails, even where ncdb would read it alike" "$r" "model.lock.json"

# ---- check: the model only grows -------------------------------------------------------------

echo "check: the model only grows against the base branch"

r="$(fixture)"; sed -i.bak '/^    role: nc.string(),/d' "$r/model/model.ts"; rm "$r/model/model.ts.bak"; rebuild_fresh "$r"; commit "$r"
expect_fail "a removed field fails, though the lock was rebuilt from nothing" "$r" "spaceMember.role"

r="$(fixture)"; sed -i.bak 's/^    role: nc.string(),/    roleName: nc.string(),/' "$r/model/model.ts"; rm "$r/model/model.ts.bak"; rebuild_fresh "$r"; commit "$r"
expect_fail "a renamed field fails, though the lock was rebuilt from nothing" "$r" "spaceMember.role"

r="$(fixture)"; sed -i.bak '/^    role: nc.string(),/d' "$r/model/model.ts"; rm "$r/model/model.ts.bak"; commit "$r"
expect_fail "a removed field fails when the lock was left alone" "$r" "spaceMember.role"

r="$(fixture)"; git -C "$r" rm -rq model; git -C "$r" commit -qm "no model"; git -C "$r" tag -f base >/dev/null
git -C "$r" checkout -q "$(git -C "$r" rev-list --max-parents=0 HEAD)" -- model; commit "$r"
expect_pass "a base branch without a lock passes: there is nothing to grow from" "$r"
if grep -q "no model/model.lock.json" <<<"$OUT"; then ok "it says the base has no lock"; else bad "it says the base has no lock ($OUT)"; fi

# ---- check: the model's identity against the base branch (NEX-223) ----------------------------

echo "check: the lock keeps the base's model id, and its model version never goes down"

BASE_ID="$(sed -n 's/^ *"id": "\([^"]*\)".*/\1/p' "$ROOT/model/model.lock.json")"
set_lock() { # <repo> <sed expression>: a hand edit of the committed lock
  sed -i.bak "$2" "$1/model/model.lock.json"; rm "$1/model/model.lock.json.bak"
}

r="$(fixture)"; set_lock "$r" "s/\"id\": \"$BASE_ID\"/\"id\": \"00000000000000000000000000000000\"/"; commit "$r"
expect_fail "a lock whose model id was edited by hand fails, though ncdb rebuilds it alike" "$r" "model id"
if grep -qF "$BASE_ID" <<<"$OUT"; then ok "it names the base's model id"; else bad "it names the base's model id ($OUT)"; fi

r="$(fixture)"; rebuild_fresh "$r"; commit "$r"
expect_fail "a lock rebuilt from nothing fails, though model.ts is unchanged: its model id is new" "$r" "model id"

r="$(fixture)"; add_field "$r" note; rebuild "$r"; commit "$r"; git -C "$r" tag -f base >/dev/null
set_lock "$r" 's/"version": 2/"version": 1/'; commit "$r"
expect_fail "a lock whose model version went down fails, though ncdb rebuilds it alike" "$r" "model version"
if grep -qF "below the base's 2" <<<"$OUT"; then ok "it names both versions"; else bad "it names both versions ($OUT)"; fi

r="$(fixture)"
check "$r" no-such-ref
if [ "$RC" != 0 ] && grep -q "no-such-ref" <<<"$OUT"; then ok "a base git cannot resolve fails, named"; else bad "a base git cannot resolve fails, named (exit $RC: $OUT)"; fi

# ---- pin -------------------------------------------------------------------------------------

echo "pin: the version Cargo.lock pins, which model/package.json must name too"

pin() { # <repo>
  RC=0
  OUT="$(cd "$1" && bash "$SCRIPT" pin 2>&1)" || RC=$?
}

r="$(fixture)"; pin "$r"
if [ "$RC" = 0 ] && [ "$OUT" = "$PIN" ]; then ok "the pin is Cargo.lock's nocentraldb version"; else bad "the pin is Cargo.lock's nocentraldb version (exit $RC: $OUT, wanted $PIN)"; fi

r="$(fixture)"; sed -i.bak "s|\"@nocentraldb/sdk\": \"$PIN\"|\"@nocentraldb/sdk\": \"0.0.1\"|" "$r/model/package.json"; rm "$r/model/package.json.bak"; pin "$r"
if [ "$RC" != 0 ] && grep -q "0.0.1" <<<"$OUT"; then ok "an SDK pin in model/package.json that differs fails, named"; else bad "an SDK pin in model/package.json that differs fails, named (exit $RC: $OUT)"; fi

r="$(fixture)"; sed -i.bak '/^name = "nocentraldb"$/,/^$/d' "$r/Cargo.lock"; rm "$r/Cargo.lock.bak"; pin "$r"
if [ "$RC" != 0 ] && grep -q "Cargo.lock" <<<"$OUT"; then ok "a Cargo.lock without nocentraldb fails"; else bad "a Cargo.lock without nocentraldb fails (exit $RC: $OUT)"; fi

# ---- install, against a stubbed cargo ---------------------------------------------------------

echo "install: ncdb at the pinned version, against a stubbed cargo"

STUB="$WORK/stub"
mkdir -p "$STUB"
# `cargo install --list --root R` prints R/installed; `cargo install ncdb --version =V ... --root R`
# logs its arguments and writes R/installed as cargo would list it (unless STUB_INSTALLS is a version
# to claim instead, to play a cargo that installed something else).
cat > "$STUB/cargo" <<'EOF'
#!/usr/bin/env bash
root=""; version=""; list=0
prev=""
for a in "$@"; do
  case "$prev" in --root) root="$a" ;; --version) version="${a#=}" ;; esac
  [ "$a" = --list ] && list=1
  prev="$a"
done
if [ "$list" = 1 ]; then cat "$root/installed" 2>/dev/null || true; exit 0; fi
printf '%s\n' "$*" >> "$STUB_LOG"
mkdir -p "$root/bin"
printf 'ncdb v%s:\n    ncdb\n' "${STUB_INSTALLS:-$version}" > "$root/installed"
EOF
chmod +x "$STUB/cargo"
export STUB_LOG="$WORK/cargo.log"

install() { # <repo> <root>
  RC=0
  : > "$STUB_LOG"
  OUT="$(cd "$1" && PATH="$STUB:$PATH" bash "$SCRIPT" install "$2" 2>&1)" || RC=$?
}

r="$(fixture)"
install "$r" "$WORK/ncdb-empty"
if [ "$RC" = 0 ] && grep -qx -- "install ncdb --version =$PIN --locked --root $WORK/ncdb-empty" "$STUB_LOG"; then
  ok "an empty root gets ncdb at exactly the pin, --locked"
else
  bad "an empty root gets ncdb at exactly the pin, --locked (exit $RC: $OUT; cargo: $(cat "$STUB_LOG"))"
fi

mkdir -p "$WORK/ncdb-cached"; printf 'ncdb v%s:\n    ncdb\n' "$PIN" > "$WORK/ncdb-cached/installed"
install "$r" "$WORK/ncdb-cached"
if [ "$RC" = 0 ] && [ ! -s "$STUB_LOG" ]; then ok "a root that holds the pin is kept"; else bad "a root that holds the pin is kept (exit $RC: $OUT; cargo: $(cat "$STUB_LOG"))"; fi

mkdir -p "$WORK/ncdb-old"; printf 'ncdb v0.0.1:\n    ncdb\n' > "$WORK/ncdb-old/installed"
install "$r" "$WORK/ncdb-old"
if [ "$RC" = 0 ] && grep -q -- "--version =$PIN" "$STUB_LOG"; then ok "a root that holds another version is reinstalled at the pin"; else bad "a root that holds another version is reinstalled at the pin (exit $RC: $OUT; cargo: $(cat "$STUB_LOG"))"; fi

STUB_INSTALLS=0.0.2 install "$r" "$WORK/ncdb-wrong"
if [ "$RC" != 0 ] && grep -q "0.0.2" <<<"$OUT"; then ok "an install that does not end at the pin fails, named"; else bad "an install that does not end at the pin fails, named (exit $RC: $OUT)"; fi

echo
echo "model-lock: $PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
