import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { test } from 'node:test';

const execFileAsync = promisify(execFile);
const root = new URL('../..', import.meta.url).pathname.replace(/^\/+([A-Za-z]):/, '$1:');
const categories = [
  'readonly_explanation',
  'single_file_bug_fix',
  'multi_file_feature',
  'test_failure_repair',
  'approval_required_action',
  'dirty_workspace',
  'long_context_compression',
  'safe_pause',
  'restart_recovery',
  'evidence_export_redaction',
];

test('Codex task matrix defines ten controlled categories and defers real invocation', async () => {
  const matrix = await readFile(join(root, 'docs', 'preview', 'codex-task-matrix.md'), 'utf8');
  for (const category of categories) assert.match(matrix, new RegExp(`\\b${category}\\b`));
  assert.match(matrix, /real invocation deferred/);
  assert.match(matrix, /P0|P1/);
});

test('matrix summarizer emits ten immutable controlled results and rejects overwrite', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-matrix-test-'));
  const evidenceRoot = join(directory, 'evidence');
  const outputPath = join(directory, 'report.json');
  try {
    const script = join(root, 'scripts', 'summarize-codex-matrix.ps1');
    await execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script,
      '-FixtureMode', '-EvidenceRoot', evidenceRoot, '-OutputPath', outputPath,
    ]);
    const report = JSON.parse((await readFile(outputPath, 'utf8')).replace(/^\uFEFF/, ''));
    assert.equal(report.totalTasks, 10);
    assert.deepEqual(report.categories.sort(), [...categories].sort());
    assert.equal(report.realInvocation, 'deferred');
    assert.equal(report.p0p1Count, 0);
    await assert.rejects(
      execFileAsync('powershell.exe', [
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script,
        '-EvidenceRoot', evidenceRoot, '-OutputPath', outputPath,
      ]),
      /refuses to overwrite|OUTPUT_EXISTS/i,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('matrix summarizer blocks P0/P1 evidence', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-matrix-p0-'));
  const evidenceRoot = join(directory, 'evidence');
  const outputPath = join(directory, 'report.json');
  try {
    const script = join(root, 'scripts', 'summarize-codex-matrix.ps1');
    await execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script,
      '-FixtureMode', '-EvidenceRoot', evidenceRoot, '-OutputPath', outputPath,
    ]);
    const first = join(evidenceRoot, '01-readonly_explanation.json');
    const evidence = JSON.parse((await readFile(first, 'utf8')).replace(/^\uFEFF/, ''));
    evidence.defectIds = ['P0-fixture-regression'];
    await writeFile(first, JSON.stringify(evidence), 'utf8');
    await assert.rejects(
      execFileAsync('powershell.exe', [
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script,
        '-EvidenceRoot', evidenceRoot, '-OutputPath', join(directory, 'p0-report.json'),
      ]),
      /P0|P1/i,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
