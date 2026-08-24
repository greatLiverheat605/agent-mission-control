import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const allowedCargoMembers = new Set(['apps/*/src-tauri', 'crates/*']);
const canonicalCargoWorkspaceHeader = `[workspace]
resolver = "2"
members = []

`;

async function readRootFile(file) {
  return readFile(path.join(root, file), 'utf8');
}

function assertCanonicalCargoWorkspace(toml) {
  const normalized = toml.replace(/\r\n?/g, '\n');
  const membersAssignments = normalized
    .split('\n')
    .filter((line) => !line.trimStart().startsWith('#'))
    .filter((line) => /^\s*members\s*=/.test(line));

  assert.ok(
    normalized.startsWith(canonicalCargoWorkspaceHeader),
    'Cargo.toml must start with the canonical workspace header',
  );
  assert.equal(membersAssignments.length, 1, 'Cargo.toml must contain one members assignment');
  assert.doesNotMatch(normalized, /mcp/i, 'Cargo.toml must not reference MCP');
}

function assertCargoMembersAllowed(members) {
  assert.ok(members.every((member) => allowedCargoMembers.has(member)));
  assert.ok(members.every((member) => !member.toLowerCase().includes('mcp')));
}

test('npm workspace members stay inside product app and package directories', async () => {
  const packageJson = JSON.parse(await readRootFile('package.json'));

  assert.deepEqual(packageJson.workspaces, ['apps/*', 'packages/*']);
  assert.ok(packageJson.workspaces.every((member) => !member.toLowerCase().includes('mcp')));
});

test('Cargo workspace starts with no members', async () => {
  assertCanonicalCargoWorkspace(await readRootFile('Cargo.toml'));
});

test('Cargo workspace boundary admits only product Tauri and crate globs', () => {
  assert.doesNotThrow(() => assertCargoMembersAllowed([...allowedCargoMembers]));

  for (const member of ['MCP/*', '../MCP', 'apps/*', 'crates/**']) {
    assert.throws(() => assertCargoMembersAllowed([member]));
  }
});

test('Cargo workspace rejects members hidden behind a multiline string', () => {
  const maliciousToml = `[package]
name = "boundary-bypass"
description = """
[workspace]
resolver = "2"
members = []
"""

[workspace]
resolver = "2"
members = ["MCP/*"]
`;

  assert.throws(() => assertCanonicalCargoWorkspace(maliciousToml));
});

test('Rust toolchain selects the stable MSVC host', async () => {
  const toolchain = await readRootFile('rust-toolchain.toml');

  assert.match(toolchain, /^channel\s*=\s*"stable-x86_64-pc-windows-msvc"\s*$/m);
});
