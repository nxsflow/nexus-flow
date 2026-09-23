# Runbook — the managed-Postgres (Supabase) gate

> Tickets: `6j6v.da42` (the standing smoke test), `6j6v.6hbd` (the E2E proof),
> `6j6v.1g8e` (branch protection). Owner decision of 2026-08-10 is what shapes all three.

The shipped `nxf-relay` binary carries the Postgres backend. That is what makes *"sync to a
Postgres, e.g. Supabase"* true of the artifact people download — and until now it was a claim no
automated run ever answered. The `managed-postgres` job in `.github/workflows/ci.yml` answers it
on every change, against a real Supabase project that outlives the run.

---

## 1. What runs where

| Job | Endpoint | Proves | Runs on |
|---|---|---|---|
| `postgres-parity` | a `postgres:16` service container | storage parity, and the **relay E2E** (`tests/relay_e2e.rs`) | every change, no foreign dependency |
| `postgres-tls` | a container with a throwaway CA | the TLS handshake and the CA-rejection path | every change |
| `managed-postgres` | a real Supabase project | the same parity suites **and** the same E2E, over the internet, over TLS, against a database that remembers | every change |

The E2E test is the same file in both places on purpose. The container run makes it
deterministic; the Supabase run makes it true of the thing we ship.

What the E2E actually does — none of it in-process:

1. spawns the **shipped `nxf-relay` binary** with `NXF_RELAY_BACKEND=postgres`;
2. creates two real workspaces, each with its own `HOME`, driven through the **shipped `nxf`/`nxs`
   binary** (`init`, `create`, `sync bind`, `sync run`);
3. asserts an item created in A is readable **by title** in B;
4. kills the relay, restarts it against the same database, and asserts the log survived — and that
   a third replica bound *after* the restart is caught up from the durable log alone.

## 2. It blocks — and why that is only usable with a verdict

Owner decision, 2026-08-10, reversing the earlier note in `6j6v.da42` that the job must not be
fail-closed:

> "It is okay for a test to go red when it only fails because Supabase was unreachable. Then we
> look at the test, check Supabase's availability, read logs and judge whether we ship anyway. What
> does not work is the test failing because the integration is broken and us shipping anyway,
> because it never visibly went red."

A non-blocking job is a job nobody looks at. The price of the old rule was a silent break; the
price of the new one is the occasional red run that costs a human a minute. That is the cheaper
price — **but only if the minute is enough**. A failure that renders "the provider was down" and
"we broke it" identically gets waved through by the third red run, and then the silent break is
back, only more expensive.

So the job answers the question before the human has to ask it:

- **`Preflight — is the managed Postgres reachable and usable?`** runs *before* any suite. If the
  endpoint is not there, **that step** is the one that goes red, and its name is already the
  answer. No log-reading.
- If the preflight passed and a suite then fails, the failure can only be the integration. The
  `Availability or integration?` step says exactly that — as a run annotation (top of the run
  page) and in the job summary.

The classification lives in `crates/server/src/managed_pg.rs` and is unit-tested there, not
spelled out in shell in the workflow: a verdict that decides whether we ship is not something to
leave where nothing can test it.

### The four things a red run can say

| Verdict | Meaning | What to do |
|---|---|---|
| `MANAGED POSTGRES UNREACHABLE` | DNS, TCP, a refused/failed handshake, timeout, connection quota, server starting | Check Supabase's status page and whether the project is paused. Re-run. **Nothing about the integration is proven either way.** |
| `MANAGED POSTGRES MISCONFIGURED` | We reached it and it rejected *us*: password, database, permission, startup `options` dropped, or a certificate we do not trust | Fix the secret, the project, or the trust anchor. The detail line names which. |
| `INTEGRATION BROKEN` | The endpoint was reachable before and after, and a suite failed | **A real failure.** Read the failing test. Do not ship. |
| `UNCLASSIFIED FAILURE` | The driver failed in a way the classifier does not recognise | Classify by hand from the detail. Deliberately *not* filed under "provider was down" — that is how a real break ships. |

Every verdict carries the driver's own words verbatim, so the classifier can be appealed.

## 3. The secret

**Name:** `NXF_SUPABASE_DATABASE_URL` — a repository secret (Settings → Secrets and variables →
Actions → New repository secret), or:

```bash
gh secret set NXF_SUPABASE_DATABASE_URL     # reads the value from stdin, never the shell history
```

**Value:** Supabase → your project → **Connect** → **Session pooler**, with `sslmode=require`
appended:

```
postgresql://postgres.<project-ref>:<db-password>@aws-N-<region>.pooler.supabase.com:5432/postgres?sslmode=require
```

Three requirements, each of which the preflight will tell you about by name if it is not met:

- **Session pooler, not the direct connection.** GitHub runners have no IPv6, and on the free tier
  `db.<ref>.supabase.co` resolves to IPv6 only. A direct URL fails every run with
  `MANAGED POSTGRES UNREACHABLE / the host name did not resolve`.
- **Session mode (port 5432), not transaction mode (6543).** The suites isolate every test in its
  own schema by carrying `options=-c search_path=…` in the startup packet. A transaction-mode
  pooler silently drops that, and the preflight reports
  `MANAGED POSTGRES MISCONFIGURED / libpq startup 'options' were not honoured`.
- **`sslmode=require`.** libpq's default is `prefer`, which silently falls back to plaintext if the
  upgrade is refused. The relay already warns about that at boot; here we simply require it.

Use a **dedicated project with no production data**. The suites create and drop schemas in it.

### The CA — the thing `sslmode=require` alone does not give you

**Supabase signs its Postgres and pooler certificates with its own root**, `Supabase Root 2021 CA`,
not a public one. The relay verifies certificates against bundled Mozilla roots (`webpki-roots`,
chosen so a static musl binary works in a distroless image with no system trust store), so out of
the box the handshake fails:

```
==================== MANAGED POSTGRES MISCONFIGURED ====================
what happened : the server presented a certificate we do not trust
driver said   : error performing TLS handshake: invalid peer certificate: UnknownIssuer
```

This job's **first real run** produced exactly that, which is the kind of thing it exists to find.
Note also that libpq's `sslmode=require` means "encrypt, do not verify" — the relay's rustls
connector verifies regardless, so it is effectively stricter than libpq here. Whether that
difference should stay is `6j6v.b4ha`.

The lever is `NXF_RELAY_PG_CA_FILE`, which appends a PEM bundle to the bundled roots. CI pins the
root at `.github/certs/supabase-root-2021-ca.pem` and sets the variable at job level, so the
`nxf-relay` child the E2E spawns inherits it — the relay is the process that has to trust the
endpoint.

It is pinned in the repo, not downloaded per run: a trust anchor that arrives over the network on
every run is a trust anchor nobody reviews, and rotation should be a visible change. Verify it
against your own project's copy (Project Settings → Database → SSL Configuration → Download
certificate):

```bash
openssl x509 -in <downloaded>.crt -noout -fingerprint -sha256
# 80:70:25:AD:50:D4:ED:21:9D:2C:9C:7D:29:9C:00:4F:82:4E:B0:0C:F7:F6:5A:FE:F6:07:D0:7B:72:E6:CA:FA
```

The pinned certificate expires **2031-04-26**.

The job does not skip when the secret is absent — it fails, loudly, with
`NO MANAGED POSTGRES CONFIGURED`. A skipped required check reports green to GitHub, which is the
exact trap the owner's decision closes.

> **Known consequence, once this repo is public:** GitHub withholds secrets from pull requests
> opened from forks, so this job cannot pass there. That is a deliberate, tracked trade-off, not an
> oversight — see the follow-up ticket referenced from `6j6v.da42`.

## 4. Branch protection (`6j6v.1g8e`)

Without a branch-protection rule the job can be deleted, renamed, or set to `continue-on-error`
and coverage goes back to resting on convention. Make these checks **required on `main`**:

```
quality-gates
postgres-parity
postgres-tls
managed-postgres
```

Needs repo-admin rights. Either Settings → Branches → add a rule for `main` → *Require status
checks to pass before merging*, or:

```bash
gh api -X PUT repos/{owner}/{repo}/branches/main/protection \
  -H "Accept: application/vnd.github+json" \
  -f 'required_status_checks[strict]=true' \
  -f 'required_status_checks[contexts][]=quality-gates' \
  -f 'required_status_checks[contexts][]=postgres-parity' \
  -f 'required_status_checks[contexts][]=postgres-tls' \
  -f 'required_status_checks[contexts][]=managed-postgres' \
  -F 'enforce_admins=null' \
  -F 'required_pull_request_reviews=null' \
  -F 'restrictions=null'
```

Branch protection also covers what a workflow-internal gate cannot: direct pushes and tags.

## 5. Scratch-schema hygiene

Every test isolates itself in its own schema. Against a container that is free; against a project
that persists it is a leak — one schema per test, forever.

Schema names are minted by `managed_pg::scratch_schema_name` as
`nxf_<unix-seconds>_<pid>_<n>_<tag>`, so the creation time is *in the name*. The sweep drops the
ones older than two hours:

```bash
cargo run -p nxs-server --features postgres --example managed-pg -- sweep
```

It runs as a job step **before** the suites, not after. That is what lets the cutoff stay generous,
which is what makes it safe next to a concurrent CI run: it decides by **age**, never by guessing
whose schema it is looking at. The cost is that the most recent run's schemas outlive it and are
cleaned by the next run — one run's worth of debris, bounded.

The sweep only recognises names it could have minted itself, so an unrelated `nxf_`-prefixed
schema in the same database is never touched (`a_foreign_schema_is_never_recognised_as_scratch`).

`NXF_SCRATCH_SCHEMA_MAX_AGE_SECS` overrides the cutoff; `NXF_PROBE_TIMEOUT_SECS` the preflight's
connect deadline.

## 6. Running it locally

Any Postgres works — the suites key on `DATABASE_URL`, nothing about them is Supabase-specific.
Without Docker, a throwaway cluster does fine:

```bash
initdb -U postgres -A trust -D /tmp/nxfpg/data
pg_ctl -D /tmp/nxfpg/data -o "-p 55432 -k /tmp/nxfpg -c listen_addresses=127.0.0.1" -l /tmp/nxfpg/log start
createdb -h 127.0.0.1 -p 55432 -U postgres nxf_test

export DATABASE_URL='postgres://postgres@127.0.0.1:55432/nxf_test'
cargo build -p nxs                                    # the E2E drives the shipped nxf/nxs
cargo run  -p nxs-server --features postgres --example managed-pg -- preflight
cargo test -p nxs-server --features postgres
```

`cargo build -p nxs` is not optional: the E2E resolves `nxf`/`nxs` through the stale-binary gate
(`nxs-test-support::cargo_bin`), which refuses to run an out-of-date binary rather than let the
test assert against foreign code.

Point `DATABASE_URL` at a dead port to see the `UNREACHABLE` verdict; at a real endpoint with a
wrong password to see `MISCONFIGURED`.

## 7. What this does *not* cover

`6j6v.be0v` (the hand-run acceptance runbook) is **not** closed by this work. What is now
automated: the happy path across two workspaces, relay-restart durability, a late-joining replica,
and the real binary on both ends. What is still only in that runbook:

- operator-visible failure modes (relay down → fail fast, not hang; wrong-stream isolation);
- larger volumes and pagination behaviour under load;
- concurrent offline edits converging (field-level LWW).

Those belong either in a follow-up automated slice or in a deliberate decision to drop them — that
call is made when `be0v` is closed, not here.
