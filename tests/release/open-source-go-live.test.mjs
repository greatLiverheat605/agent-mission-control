import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);
const root = new URL('../..', import.meta.url).pathname.replace(/^\/+([A-Za-z]):/, '$1:');

async function distributionDecision(argumentsText) {
  const script = join(root, 'scripts', 'release-distribution.ps1').replaceAll("'", "''");
  const result = await execFileAsync('powershell.exe', [
    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command',
    `. '${script}'; Resolve-ReleaseDistribution ${argumentsText} | ConvertTo-Json -Compress`,
  ]);
  return JSON.parse(result.stdout.replace(/^\uFEFF/, '').trim());
}

async function initRepository(directory) {
  await execFileAsync('git', ['init', '-b', 'main'], { cwd: directory });
  await execFileAsync('git', ['config', 'user.name', 'Open Source Audit Test'], { cwd: directory });
  await execFileAsync('git', ['config', 'user.email', 'audit@example.test'], { cwd: directory });
}

test('oss-personal makes signing and clean VM optional while real evidence remains mandatory', async () => {
  const ready = await distributionDecision("-Distribution oss-personal -FailureCount 0 -SigningStatus deferred-ci -EvidenceReady $false -RealInvocationStatus authorized-real -CleanVmStatus blocked");
  assert.equal(ready.releaseReady, true);
  assert.deepEqual(ready.formalReleaseBlockedBy, []);
  assert.deepEqual(ready.optionalEnhancements, ['signing-credentials', 'clean-vm']);

  const deferred = await distributionDecision("-Distribution oss-personal -FailureCount 0 -SigningStatus deferred-ci -EvidenceReady $false -RealInvocationStatus deferred -CleanVmStatus blocked");
  assert.equal(deferred.releaseReady, false);
  assert.deepEqual(deferred.formalReleaseBlockedBy, ['real-provider-invocation']);
});

test('commercial distribution preserves production signing evidence and clean VM blockers', async () => {
  const decision = await distributionDecision("-Distribution commercial -FailureCount 0 -SigningStatus deferred-ci -EvidenceReady $false -RealInvocationStatus authorized-real -CleanVmStatus blocked");
  assert.equal(decision.releaseReady, false);
  assert.deepEqual(decision.formalReleaseBlockedBy, ['signing-credentials', 'clean-vm']);
  assert.deepEqual(decision.optionalEnhancements, []);
});

test('release gate and review matrix publish the selected distribution decision', async () => {
  const verify = await readFile(join(root, 'scripts', 'verify-codex-release.ps1'), 'utf8');
  const matrix = await readFile(join(root, 'scripts', 'build-p1-finding-matrix.ps1'), 'utf8');
  assert.match(verify, /ValidateSet\('commercial',\s*'oss-personal'\)/);
  assert.match(verify, /\$Distribution\s*=\s*'oss-personal'/);
  assert.match(verify, /Resolve-ReleaseDistribution/);
  for (const field of ['distribution', 'formalReleaseBlockedBy', 'optionalEnhancements', 'releaseReady']) {
    assert.match(verify, new RegExp(`${field}\\s*=`));
    assert.match(matrix, new RegExp(`${field}\\s*=`));
  }
});

test('open source metadata, publication boundaries, and operator guidance stay explicit', async () => {
  const license = await readFile(join(root, 'LICENSE'), 'utf8');
  const cargo = await readFile(join(root, 'Cargo.toml'), 'utf8');
  const packageManifest = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
  const ignore = await readFile(join(root, '.gitignore'), 'utf8');
  const readme = await readFile(join(root, 'README.md'), 'utf8');
  const security = await readFile(join(root, 'SECURITY.md'), 'utf8');
  const fixtureNotice = await readFile(join(root, 'fixtures', 'agents', 'README.md'), 'utf8');
  const runbook = await readFile(join(root, 'docs', 'release', 'formal-release-runbook.md'), 'utf8');

  assert.match(license, /MIT License/);
  assert.match(license, /Agent Mission Control contributors/);
  assert.match(cargo, /license\s*=\s*"MIT"/);
  assert.equal(packageManifest.license, 'MIT');
  for (const ignored of ['/artifacts/', 'test-results/', 'testResults.xml', '/.codex/', 'target/', 'node_modules/']) {
    assert.match(ignore, new RegExp(ignored.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')));
  }
  assert.match(ignore, /scripts.*docs.*fixtures|scripts[\s\S]*docs[\s\S]*fixtures/i);
  for (const topic of ['windows-setup.md', 'cargo clippy', 'SmartScreen', 'MIT', 'disclaimer']) assert.match(readme, new RegExp(topic, 'i'));
  assert.match(security, /privately|private/i);
  assert.match(fixtureNotice, /synthetic|fake/i);
  assert.match(runbook, /oss-personal/);
  for (const provider of ['SignPath', 'Trusted Signing', 'Certum']) assert.match(runbook, new RegExp(provider, 'i'));
});

test('open source audit reports pending privacy hits and historical risk without rewriting files', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'amc-open-source-audit-'));
  try {
    await initRepository(directory);
    await writeFile(join(directory, 'history.txt'), 'legacy build root G:\\legacy\\build\n', 'utf8');
    await execFileAsync('git', ['add', 'history.txt'], { cwd: directory });
    await execFileAsync('git', ['commit', '-m', 'fixture history'], { cwd: directory });
    await writeFile(join(directory, 'pending.txt'), 'profile C:\\Users\\Administrator\\project\nendpoint http://192.168.12.4/api\n', 'utf8');

    const outputRoot = join(directory, 'audit-output');
    await execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'audit-open-source.ps1'),
      '-RepositoryRoot', directory, '-OutputRoot', outputRoot, '-RunId', 'privacy-review',
      '-UserNames', 'Administrator', '-MachineNames', 'BUILDHOST',
    ]);
    const report = JSON.parse((await readFile(join(outputRoot, 'privacy-review', 'open-source-audit.json'), 'utf8')).replace(/^\uFEFF/, ''));
    assert.equal(report.status, 'review-required');
    assert.deepEqual(report.pendingFiles, ['pending.txt']);
    assert.ok(report.privacyFindings.some(({ category }) => category === 'user-profile-path'));
    assert.ok(report.privacyFindings.some(({ category }) => category === 'intranet-address'));
    assert.ok(report.history.highRiskMatchCount >= 1);
    assert.equal(report.secretScan.status, 'pass');
    assert.equal(await readFile(join(directory, 'pending.txt'), 'utf8'), 'profile C:\\Users\\Administrator\\project\nendpoint http://192.168.12.4/api\n');
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('open source audit fails closed when the existing secret scanner finds a corpus value', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'amc-open-source-secret-'));
  try {
    await initRepository(directory);
    await writeFile(join(directory, 'safe.txt'), 'safe\n', 'utf8');
    await execFileAsync('git', ['add', 'safe.txt'], { cwd: directory });
    await execFileAsync('git', ['commit', '-m', 'fixture baseline'], { cwd: directory });
    await writeFile(join(directory, 'pending.txt'), 'ghp_123456789012345678901234567890123456\n', 'utf8');

    const outputRoot = join(directory, 'audit-output');
    await assert.rejects(execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'audit-open-source.ps1'),
      '-RepositoryRoot', directory, '-OutputRoot', outputRoot, '-RunId', 'secret-rejected',
    ]));
    const report = JSON.parse((await readFile(join(outputRoot, 'secret-rejected', 'open-source-audit.json'), 'utf8')).replace(/^\uFEFF/, ''));
    assert.equal(report.status, 'fail');
    assert.equal(report.secretScan.status, 'fail');
    assert.ok(report.secretScan.matches.length > 0);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
