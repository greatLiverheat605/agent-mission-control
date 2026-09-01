import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { spawn } from 'node:child_process';

const root = new URL('../..', import.meta.url).pathname.replace(/^\/+([A-Za-z]):/, '$1:');
const scanner = join(root, 'scripts', 'scan-secrets.ps1');
const corpus = join(root, 'fixtures', 'agents', 'secret-corpus.json');

function runPowerShell(args) {
  return new Promise((resolve, reject) => {
    const child = spawn('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', scanner, ...args], { cwd: root });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => { stdout += chunk; });
    child.stderr.on('data', (chunk) => { stderr += chunk; });
    child.on('error', reject);
    child.on('close', (code) => resolve({ code, stdout, stderr }));
  });
}

test('secret scanner fails on raw corpus values and emits no secret material', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'mission-security-'));
  await writeFile(join(dir, 'leak.log'), 'Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.example.signature', 'utf8');
  const result = await runPowerShell(['-Root', dir, '-Corpus', corpus]);
  assert.equal(result.code, 1);
  assert.match(result.stdout, /"status"\s*:\s*"fail"/i);
  assert.doesNotMatch(result.stdout, /eyJhbGciOiJIUzI1NiJ9/);
});

test('scanner accepts redacted artifacts', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'mission-security-'));
  await writeFile(join(dir, 'diagnostic.json'), '{"authorization":"[REDACTED:token:abc123]"}', 'utf8');
  const result = await runPowerShell(['-Root', dir, '-Corpus', corpus]);
  assert.equal(result.code, 0, result.stderr);
});

test('scanner rejects Anthropic keys with hyphenated prefix', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'mission-security-'));
  await writeFile(join(dir, 'anthropic.txt'), 'key=sk-ant-api03-abcdefghijklmnopqrstuvwxyz', 'utf8');
  const result = await runPowerShell(['-Root', dir, '-Corpus', corpus]);
  assert.equal(result.code, 1);
  assert.match(result.stdout, /anthropic_key/);
  assert.doesNotMatch(result.stdout, /sk-ant-api03-abcdefghijklmnopqrstuvwxyz/);
});

test('scanner reports an external corpus when the scan root is deeper', async () => {
  const parent = await mkdtemp(join(tmpdir(), 'mission-security-root-'));
  const dir = join(parent, 'nested', 'root', 'with', 'a', 'deliberately', 'long', 'path');
  await mkdir(dir, { recursive: true });
  await writeFile(join(dir, 'clean.txt'), 'no credentials here', 'utf8');
  const result = await runPowerShell(['-Root', dir, '-Corpus', corpus]);
  assert.equal(result.code, 0, result.stderr);
  const report = JSON.parse(result.stdout.replace(/^\uFEFF/, ''));
  assert.equal(report.status, 'pass');
  assert.match(report.corpus, /secret-corpus\.json$/i);
});
