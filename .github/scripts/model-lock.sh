#!/usr/bin/env bash
# CI's model lock step (NEX-91, RFC 0001 "The model": "Building it" and "The model only grows").
#
# nexus-flow's NoCentralDB model is model/model.ts; model/model.lock.json is what `ncdb model build`
# writes from it, and the Rust build reads only that lock. This script keeps the two honest:
#
#   pin              Prints the NoCentralDB version the repository pins: Cargo.lock's `nocentraldb`.
#                    Fails when model/package.json pins `@nocentraldb/sdk` at another version, so the
#                    SDK that builds the lock and the SDK that reads it are one version.
#   install <root>   Puts `ncdb` at the pinned version into <root>/bin with `cargo install --locked`,
#                    or keeps the one there when `cargo install --list` says it is that version, and
#                    fails unless it ends at the pin. CI caches <root> under the pin.
#   check <base>     Run from the repository root, with `ncdb`, Node 24 or later and model/node_modules
#                    in place. Three checks:
#                      1. the rebuild: `ncdb model build` over a scratch copy of the committed lock
#                         must write it byte for byte. A model.ts edited without a rebuild, or a lock
#                         edited by hand, fails here.
#                      2. the model only grows: `ncdb model build` over a scratch copy of the lock at
#                         <base> must accept model.ts, so no field or collection on the base branch is
#                         removed or renamed, even when the PR rebuilt its lock from nothing. A base
#                         without a lock passes with a notice. A base git cannot resolve fails.
#                      3. the lock is the base's model (NEX-223): its model id is the base lock's,
#                         and its model version is no lower than the base lock's. ncdb rebuilds a
#                         hand-edited id or a lowered version byte for byte and accepts model.ts over
#                         the base lock, so checks 1 and 2 pass both; a lock rebuilt from nothing has a
#                         new id and fails here. Skipped, as 2 is, on a base without a lock.
#                    All three run, any one fails the step, and none touches the working tree.
#
# Why `ncdb model build` on a scratch copy for check 2 and not `ncdb model check --lock <base>`:
# `check` also fails when the lock is out of date, so against the base lock it fails every PR that
# adds a field. `build` runs the same additivity rule and writes only the scratch copy. And why the
# base lock and not a comparison with the PR's lock rebuilt from it: each rebuild raises the model
# version, so a branch that rebuilt twice holds version 3 where a rebuild from the base writes 2.
#
# Tested by tests/model-lock.test.sh, with the real ncdb (run by ci.yml's lint job).
set -euo pipefail

MODEL_DIR=model
LOCK="$MODEL_DIR/model.lock.json"

die() { printf '::error::%s\n' "$1"; exit 1; }

# Sets PIN. Not called in a subshell, so its ::error:: reaches the log.
read_pin() {
  local cargo_pin sdk_pin
  [ -f Cargo.lock ] || die "no Cargo.lock here: run from the repository root"
  cargo_pin="$(awk '
    /^\[\[package\]\]$/ { found = 0 }
    /^name = "nocentraldb"$/ { found = 1; next }
    found && /^version = / { gsub(/"/, "", $3); print $3; exit }
  ' Cargo.lock)"
  [ -n "$cargo_pin" ] || die "Cargo.lock has no nocentraldb package: there is no pin to run ncdb at"
  sdk_pin="$(sed -n 's/^ *"@nocentraldb\/sdk": *"\([^"]*\)".*/\1/p' "$MODEL_DIR/package.json")"
  [ "$sdk_pin" = "$cargo_pin" ] ||
    die "$MODEL_DIR/package.json pins @nocentraldb/sdk at '$sdk_pin', Cargo.lock pins nocentraldb at $cargo_pin: move both in one PR"
  PIN="$cargo_pin"
}

installed_ncdb() { # <root>
  cargo install --list --root "$1" 2>/dev/null | sed -n 's/^ncdb v\([^:]*\):$/\1/p'
}

install() { # <root>
  local root="$1" version got
  read_pin
  version="$PIN"
  if [ "$(installed_ncdb "$root")" = "$version" ]; then
    echo "ncdb $version is already in $root"
  else
    cargo install ncdb --version "=$version" --locked --root "$root"
  fi
  got="$(installed_ncdb "$root")"
  [ "$got" = "$version" ] || die "ncdb in $root is '$got', not the pinned $version"
}

# Prints "<id> <version>" of a lock's model, with the Node that ncdb needs anyway. Fails on a lock
# without a model id or a whole-number model version, which ncdb refuses too.
model_identity() { # <lock>
  # shellcheck disable=SC2016 # the backticks are JavaScript's, for node
  node -e '
    const { id, version } = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).model;
    if (typeof id !== "string" || !Number.isSafeInteger(version)) process.exit(1);
    console.log(`${id} ${version}`);
  ' "$1"
}

check() { # <base>
  local base="$1" base_commit scratch failed=0 head_identity="" base_identity="" head_id head_version base_id base_version
  base_commit="$(git rev-parse --verify --quiet "$base^{commit}")" ||
    die "the base '$base' is no commit git can resolve: fetch it before the check"
  [ -f "$LOCK" ] || die "$LOCK is missing"
  scratch="$(mktemp -d "${TMPDIR:-/tmp}/model-lock.XXXXXX")"
  # shellcheck disable=SC2064 # expand now: the path is fixed, and the trap outlives the local
  trap "rm -rf '$scratch'" EXIT

  echo "1. the committed lock is what ncdb model build writes from $MODEL_DIR/model.ts"
  cp "$LOCK" "$scratch/rebuilt.json"
  if ! (cd "$MODEL_DIR" && ncdb model build --lock "$scratch/rebuilt.json"); then
    printf '::error file=%s::ncdb model build refuses model.ts against the committed lock (above)\n' "$LOCK"
    failed=1
  elif ! cmp -s "$LOCK" "$scratch/rebuilt.json"; then
    diff -u --label "$LOCK (committed)" --label "$LOCK (rebuilt)" "$LOCK" "$scratch/rebuilt.json" || true
    printf '::error file=%s::%s is not what ncdb model build writes from model.ts: run "npm ci && ncdb model build" in %s/ and commit the lock\n' "$LOCK" "$LOCK" "$MODEL_DIR"
    failed=1
  fi

  echo "2. the model only grows: model.ts against the lock at $base ($base_commit)"
  if git cat-file -e "$base_commit:$LOCK" 2>/dev/null; then
    git show "$base_commit:$LOCK" > "$scratch/base.json"
    # Read before the build below raises the scratch copy's version. A lock ncdb cannot read has
    # failed check 1 or check 2 already, so a failed read only skips check 3.
    head_identity="$(model_identity "$LOCK" 2>/dev/null)" || failed=1
    base_identity="$(model_identity "$scratch/base.json" 2>/dev/null)" || failed=1
    if ! (cd "$MODEL_DIR" && ncdb model build --lock "$scratch/base.json"); then
      printf '::error file=%s::model.ts is not an additive change of the lock on %s (above): a field or collection, once on the base branch, is never renamed or removed (RFC 0001, "The model only grows")\n' "$MODEL_DIR/model.ts" "$base"
      failed=1
    fi

    echo "3. the lock is the base's model: the same model id, and a model version no lower than the base's"
    if [ -n "$head_identity" ] && [ -n "$base_identity" ]; then
      read -r head_id head_version <<<"$head_identity"
      read -r base_id base_version <<<"$base_identity"
      if [ "$head_id" != "$base_id" ]; then
        printf '::error file=%s::the model id is %s where the lock on %s has %s: a model keeps its id for good, so edit model.ts and rebuild over the committed lock, never start a new one (RFC 0001, "The model only grows")\n' "$LOCK" "$head_id" "$base" "$base_id"
        failed=1
      fi
      if [ "$head_version" -lt "$base_version" ]; then
        printf "::error file=%s::the model version is %s, below the base's %s on %s: the version only rises (RFC 0001, \"The model only grows\")\n" "$LOCK" "$head_version" "$base_version" "$base"
        failed=1
      fi
    fi
  else
    printf '::notice::%s has no %s: there is no older model to grow from\n' "$base" "$LOCK"
  fi

  return "$failed"
}

case "${1:-}" in
  pin) read_pin; printf '%s\n' "$PIN" ;;
  install) [ "$#" = 2 ] || die "usage: $0 install <root>"; install "$2" ;;
  check) [ "$#" = 2 ] || die "usage: $0 check <base>"; check "$2" ;;
  *) die "usage: $0 pin | install <root> | check <base>" ;;
esac
