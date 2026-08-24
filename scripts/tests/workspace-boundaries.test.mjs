import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const allowedCargoMembers = new Set(['apps/*/src-tauri', 'crates/*']);

async function readRootFile(file) {
  return readFile(path.join(root, file), 'utf8');
}

function cargoWorkspaceMembers(toml) {
  const workspace = toml.match(/^\[workspace\]\s*$([\s\S]*?)(?=^\[|(?![\s\S]))/m);
  assert.ok(workspace, 'Cargo.toml must contain a [workspace] section');

  const members = workspace[1].match(/^\s*members\s*=\s*\[([\s\S]*?)\]/m);
  assert.ok(members, '[workspace] must declare members as an array');

  const values = [...members[1].matchAll(/"([^"]+)"/g)].map((match) => match[1]);
  const remainder = members[1].replace(/"[^"]+"|[\s,]/g, '');
  assert.equal(remainder, '', 'workspace members must be simple quoted strings');
  return values;
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
  const members = cargoWorkspaceMembers(await readRootFile('Cargo.toml'));

  assertCargoMembersAllowed(members);
  assert.deepEqual(members, []);
});

test('Cargo workspace boundary admits only product Tauri and crate globs', () => {
  assert.doesNotThrow(() => assertCargoMembersAllowed([...allowedCargoMembers]));

  for (const member of ['MCP/*', '../MCP', 'apps/*', 'crates/**']) {
    assert.throws(() => assertCargoMembersAllowed([member]));
  }
});

test('Rust toolchain selects the stable MSVC host', async () => {
  const toolchain = await readRootFile('rust-toolchain.toml');

  assert.match(toolchain, /^channel\s*=\s*"stable-x86_64-pc-windows-msvc"\s*$/m);
});
