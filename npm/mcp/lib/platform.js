'use strict';

// Host → canonical platform key. Mirrors install.sh's `platform_for`: the shim is a componentwise
// consumer of release/platforms, the single source of truth for the four shipped targets.
// `cargo xtask platforms check` asserts the mapping below covers every canonical os/arch token,
// so a 5th target or a rename fails CI here.
//
// A THIRD SIBLING USED TO BE NAMED HERE — the updater Lambda's normalizeOs/normalizeArch. It was
// checked by the same gate until the Lambda left this repo (6j6v.sgfr); it is now in
// nxsflow-landing-page and is NOT checked by anything. See release/platforms' own header: a fifth
// target has to be carried over there by hand, and this file is not where that is tracked.

// uname os token → canonical os.
function osFor(unameS) {
  switch (unameS) {
    case 'Darwin':
      return 'darwin';
    case 'Linux':
      return 'linux';
    default:
      return null;
  }
}

// uname machine token → canonical arch (forgiving on synonyms).
function archFor(unameM) {
  switch (unameM) {
    case 'arm64':
    case 'aarch64':
      return 'aarch64';
    case 'x86_64':
    case 'amd64':
      return 'x86_64';
    default:
      return null;
  }
}

// platformFor(unameS, unameM) → "<os>-<arch>" for a shipped target, else null.
function platformFor(unameS, unameM) {
  const os = osFor(unameS);
  const arch = archFor(unameM);
  if (!os || !arch) return null;
  return `${os}-${arch}`;
}

// Node's process.platform → the uname `-s` token platformFor understands.
function unameSFor(nodePlatform) {
  switch (nodePlatform) {
    case 'darwin':
      return 'Darwin';
    case 'linux':
      return 'Linux';
    default:
      return nodePlatform; // unrecognised → platformFor returns null → detectPlatform throws
  }
}

// Node's process.arch → a uname `-m` token platformFor understands ('x64' → 'x86_64'; 'arm64'
// passes through since archFor already accepts it).
function unameMFor(nodeArch) {
  return nodeArch === 'x64' ? 'x86_64' : nodeArch;
}

// Resolve the platform key: the NXF_PLATFORM override (escape hatch / testing) wins; otherwise
// derive from the node os/arch tokens. Throws a clear message on an unsupported host rather than
// letting a 404 surface three steps later.
function detectPlatform(env, nodePlatform = process.platform, nodeArch = process.arch) {
  if (env && env.NXF_PLATFORM) return env.NXF_PLATFORM;
  const platform = platformFor(unameSFor(nodePlatform), unameMFor(nodeArch));
  if (!platform) {
    throw new Error(
      `unsupported platform: ${nodePlatform}/${nodeArch}. Supported: {darwin,linux}-{aarch64,x86_64}.`
    );
  }
  return platform;
}

module.exports = { platformFor, detectPlatform };
