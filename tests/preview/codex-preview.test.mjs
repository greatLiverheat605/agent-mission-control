import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { test } from 'node:test';

const execFileAsync = promisify(execFile);
const root = new URL('../..', import.meta.url).pathname.replace(/^\/+([A-Za-z]):/, '$1:');

test('Codex preview protocol fixes the pilot gates and privacy defaults', async () => {
  const protocol = await readFile(join(root, 'docs', 'preview', 'codex-study-protocol.md'), 'utf8');
  const privacy = await readFile(join(root, 'docs', 'preview', 'consent-and-privacy.md'), 'utf8');
  const severity = await readFile(join(root, 'docs', 'preview', 'issue-severity.md'), 'utf8');
  assert.match(protocol, /80%/);
  assert.match(protocol, /90%/);
  assert.match(protocol, /10 seconds/);
  assert.match(protocol, /real invocation deferred/);
  assert.match(privacy, /telemetry.*off/i);
  assert.match(privacy, /active|主动/i);
  assert.match(severity, /P0/);
  assert.match(severity, /P1/);
  assert.match(severity, /P2/);
});

test('preview summarizer reports passing five-person fixture and refuses overwrite', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-preview-test-'));
  const samplesPath = join(directory, 'samples');
  const outputPath = join(directory, 'report.json');
  try {
    const script = join(root, 'scripts', 'summarize-codex-preview.ps1');
    await execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script,
      '-FixtureMode', '-SamplesPath', samplesPath, '-OutputPath', outputPath,
    ]);
    const report = JSON.parse((await readFile(outputPath, 'utf8')).replace(/^\uFEFF/, ''));
    assert.equal(report.participants, 5);
    assert.equal(report.firstLaunchRate, 80);
    assert.equal(report.stateRecognitionRate, 100);
    assert.equal(report.p0p1Count, 0);
    assert.equal(report.telemetryEnabled, false);
    assert.ok(report.slices.length >= 3);
    await assert.rejects(
      execFileAsync('powershell.exe', [
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script,
        '-SamplesPath', samplesPath, '-OutputPath', outputPath,
      ]),
      /OUTPUT_EXISTS/i,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('preview summarizer blocks P0/P1 pilot evidence', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-preview-p0-'));
  const samplesPath = join(directory, 'samples');
  const outputPath = join(directory, 'report.json');
  try {
    const script = join(root, 'scripts', 'summarize-codex-preview.ps1');
    await execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script,
      '-FixtureMode', '-SamplesPath', samplesPath, '-OutputPath', outputPath,
    ]);
    const first = join(samplesPath, 'participant-01.json');
    const sample = JSON.parse((await readFile(first, 'utf8')).replace(/^\uFEFF/, ''));
    sample.issueSeverity = 'P1';
    await writeFile(first, JSON.stringify(sample), 'utf8');
    await assert.rejects(
      execFileAsync('powershell.exe', [
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script,
        '-SamplesPath', samplesPath, '-OutputPath', join(directory, 'p0-report.json'),
      ]),
      /P0|P1/i,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
