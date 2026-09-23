# @nexus-flow/mcp

A tiny **runner shim** that lets an MCP host (Claude Desktop, Cursor, Windsurf, …) start the
[nexus-flow](https://nxsflow.com/) MCP server **without `nxs` installed first**.

It fetches the signed `nxs` prebuilt for your platform, **verifies it (sha256 + minisign)**,
caches it, and execs `nxs mcp serve` — passing your arguments through.

## Use it in a host config

```json
{
  "mcpServers": {
    "nxs": {
      "command": "npx",
      "args": ["-y", "@nexus-flow/mcp", "--", "--workspace", "/absolute/path/to/your/project"]
    }
  }
}
```

Everything after `--` is passed straight to `nxs mcp serve` (e.g. `--workspace`, `--actor`). If
you already have `nxs` on `PATH`, point the host at it directly instead — this shim exists for the
"not installed at all" case (`nxs mcp install` writes the direct entry; `nxs mcp install --runner
npx` writes the entry above).

## Security

The shim **never execs an unverified binary**. Two gates, both mandatory, both fail-closed:

- **sha256** (integrity) — the `.sha256` sidecar must match the downloaded bytes.
- **minisign** (authenticity) — a prehashed Ed25519/BLAKE2b-512 signature is verified against the
  public key **embedded in this package** (the same key `nxs` and `install.sh` use). A tampered
  tarball, a wrong key, a legacy (un-prehashed) signature, or a missing signature all abort before
  anything runs. There is **no** insecure-bootstrap escape hatch here.

Verification uses only Node's built-in `crypto` — this package has **zero runtime dependencies**.

Only bytes that passed **both** gates are written into the cache (verify-before-cache, via an atomic
temp→rename). A cached binary is therefore trusted and exec'd on later launches **without**
re-verification, so the integrity of the cache directory rests on filesystem permissions — the cache
lives under your per-user cache dir (or `NXS_MCP_CACHE_DIR`); keep it user-writable only. Downloads
are bounded (per-request idle timeout, a hard byte cap, and a decompression-size cap) so a stalled or
malicious origin fails fast instead of hanging or exhausting memory before the gate.

## Configuration (env, matching `install.sh`)

| Variable            | Default                     | Meaning                                      |
| ------------------- | --------------------------- | -------------------------------------------- |
| `NXF_CHANNEL`       | `stable`                    | Promotion ring (`stable` / `beta` / `alpha`) |
| `NXF_VERSION`       | *(newest in channel)*       | Pin an exact version                         |
| `NXF_BASE_URL`      | `https://nxsflow.com/nxs`   | Artifact origin                              |
| `NXF_PLATFORM`      | *(auto from os/arch)*       | Override host detection                      |
| `NXS_MCP_CACHE_DIR` | OS cache dir                | Where the verified `nxs` is cached           |

The first launch downloads and verifies (a few seconds); subsequent launches use the cache and
start instantly — and work offline.

## Platforms

`{darwin,linux}-{aarch64,x86_64}`. Windows support is tracked upstream.

Requires Node ≥ 18.
