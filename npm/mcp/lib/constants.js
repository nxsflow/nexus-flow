'use strict';

// Embedded minisign PUBLIC key — safe to ship in a world-readable package (it verifies, never
// signs). MUST stay byte-identical to install.sh's EMBEDDED_MINISIGN_PUBKEY and the release
// pipeline's vars.NXF_MINISIGN_PUBKEY; the pubkey cross-check in .github/workflows/install-sh-ci.yml
// fails CI on drift. Rotation = update this one line (and install.sh's, and the repo var).
const EMBEDDED_MINISIGN_PUBKEY = 'RWRnRFdblE2/DEmkBWpMPqXYele5JoNtQubF+N1Ryp7msJgq7HKn2Ola';

// Public origin serving /latest, /download/*, and the sidecars (install.sh's NXF_BASE_URL default).
const DEFAULT_BASE_URL = 'https://nxsflow.com/nxs';

// Default promotion ring — the most-stable of release/channels (single source of truth). `cargo
// xtask channels check` asserts this equals the canonical most-stable channel. The set of shipped
// platform keys is not duplicated here: lib/platform.js maps the os/arch tokens componentwise (like
// install.sh's platform_for), and `cargo xtask platforms check` guards that mapping.
const DEFAULT_CHANNEL = 'stable';

module.exports = { EMBEDDED_MINISIGN_PUBKEY, DEFAULT_BASE_URL, DEFAULT_CHANNEL };
