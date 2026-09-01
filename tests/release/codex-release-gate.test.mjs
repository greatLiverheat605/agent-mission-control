import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { test } from 'node:test';
import { createHash } from 'node:crypto';

const execFileAsync = promisify(execFile);
const root = new URL('../..', import.meta.url).pathname.replace(/^\/+([A-Za-z]):/, '$1:');

test('Codex release checklist preserves the scope boundary without local plan files', async () => {
  const checklist = await readFile(join(root, 'docs', 'release', 'codex-release-checklist.md'), 'utf8');
  for (const gate of ['security', 'recovery', 'soak', 'data lifecycle', 'signed update', 'task matrix', 'preview']) assert.match(checklist, new RegExp(gate, 'i'));
  assert.match(checklist, /Codex-First/i);
  assert.match(checklist, /Claude.*deferred/i);
});

test('SBOM generator emits a hashed, redacted report without overwriting', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-sbom-test-'));
  const artifact = join(directory, 'fixture.exe');
  const output = join(directory, 'sbom.json');
  try {
    await (await import('node:fs/promises')).writeFile(artifact, 'fixture-artifact', 'utf8');
    const script = join(root, 'scripts', 'generate-sbom.ps1');
    await execFileAsync('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script, '-ArtifactPath', artifact, '-OutputPath', output, '-Version', '0.1.0']);
    const report = JSON.parse((await readFile(output, 'utf8')).replace(/^\uFEFF/, ''));
    assert.equal(report.schema, 'mission-control-sbom-v1');
    assert.match(report.artifactSha256, /^[a-f0-9]{64}$/);
    assert.ok(report.dependencies.length > 0);
    await assert.rejects(execFileAsync('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script, '-ArtifactPath', artifact, '-OutputPath', output, '-Version', '0.1.0']), /OUTPUT_EXISTS/i);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('release gate report is explicit about controlled mode and Claude deferral', async () => {
  const gate = await readFile(join(root, 'scripts', 'verify-codex-release.ps1'), 'utf8');
  for (const command of ['generate-sbom', 'security-gate', 'run-crash-matrix', 'run-codex-soak', 'summarize-codex-matrix', 'summarize-codex-preview', 'package:smoke']) assert.match(gate, new RegExp(command.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')));
  assert.match(gate, /real.*deferred/i);
  assert.match(gate, /Claude.*deferred/i);
  assert.match(gate, /PASS_INTERNAL_PREVIEW/);
  assert.match(gate, /formalReleaseBlockedBy/);
  assert.match(gate, /exit 1|throw/i);
});

test('controlled evidence never reports release readiness and skipped runners fail closed', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-evidence-gate-'));
  try {
    const matrixScript = join(root, 'scripts', 'summarize-codex-matrix.ps1');
    const matrixEvidence = join(directory, 'matrix-evidence');
    const matrixReport = join(directory, 'matrix-report.json');
    await execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', matrixScript,
      '-FixtureMode', '-EvidenceRoot', matrixEvidence, '-OutputPath', matrixReport,
    ]);
    const matrix = JSON.parse((await readFile(matrixReport, 'utf8')).replace(/^\uFEFF/, ''));
    assert.equal(matrix.evidenceGrade, 'controlled-deferred');
    assert.equal(matrix.releaseReady, false);

    const soakReport = join(directory, 'soak-report.json');
    await assert.rejects(
      execFileAsync('powershell.exe', [
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'run-codex-soak.ps1'),
        '-Root', root, '-OutputPath', soakReport, '-SkipTests',
      ]),
      /error|exit code|failed/i,
    );
    const soak = JSON.parse((await readFile(soakReport, 'utf8')).replace(/^\uFEFF/, ''));
    assert.equal(soak.status, 'fail');
    assert.equal(soak.releaseReady, false);
  } finally {
    if (!process.env.CODEX_DEBUG_KEEP) await rm(directory, { recursive: true, force: true });
  }
});

test('release hardening keeps schema, workflow, hash, and run-id contracts explicit', async () => {
  const releaseWorkflow = await readFile(join(root, '.github', 'workflows', 'windows-release.yml'), 'utf8');
  const ciWorkflow = await readFile(join(root, '.github', 'workflows', 'windows-ci.yml'), 'utf8');
  const verify = await readFile(join(root, 'scripts', 'verify-codex-release.ps1'), 'utf8');
  const sbom = await readFile(join(root, 'scripts', 'generate-sbom.ps1'), 'utf8');
  const scan = await readFile(join(root, 'scripts', 'scan-secrets.ps1'), 'utf8');
  const matrix = await readFile(join(root, 'scripts', 'build-p1-finding-matrix.ps1'), 'utf8');

  assert.match(releaseWorkflow, /fixture-selfcheck/);
  assert.match(releaseWorkflow, /unsigned-rejection-selfcheck/);
  assert.match(releaseWorkflow, /Verify full Codex release gate/);
  assert.match(releaseWorkflow, /steps\.release-gate\.outcome/);
  assert.match(ciWorkflow, /pretest:e2e/);
  assert.match(ciWorkflow, /playwright test/);
  assert.match(verify, /outputPath/);
  assert.doesNotMatch(verify, /\*>\s*\$null/);
  assert.match(verify, /manifest\/provenance\/sbom hash consistency/);
  assert.match(verify, /task matrix evidence is empty/);
  assert.match(verify, /round6Verification/);
  assert.match(verify, /same-name evidence mismatch/);
  assert.match(sbom, /checksum/);
  assert.match(scan, /detection = 'corpus\+patterns'/);
  for (const item of ['R4-1', 'R4-2', 'R4-3', 'R4-4', 'R4-5', 'R4-6', 'F-011', 'F-012', 'F-013', 'F-014', 'F-024', 'F-036', 'F-037', 'F-038']) {
    assert.match(matrix, new RegExp(item.replace('-', '\\-')));
  }
});

test('empty task and preview evidence fail closed', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-empty-evidence-'));
  try {
    const emptyTasks = join(directory, 'tasks');
    const emptySamples = join(directory, 'samples');
    await mkdir(emptyTasks);
    await mkdir(emptySamples);
    await assert.rejects(execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'summarize-codex-matrix.ps1'),
      '-EvidenceRoot', emptyTasks, '-OutputPath', join(directory, 'tasks-report.json'),
    ]), /MATRIX_COUNT_INVALID|EVIDENCE/i);
    await assert.rejects(execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'summarize-codex-preview.ps1'),
      '-SamplesPath', emptySamples, '-OutputPath', join(directory, 'preview-report.json'),
    ]), /PILOT_COUNT_INVALID|SAMPLES/i);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('Round 6 external prerequisite scripts are fail-closed and explicitly marked', async () => {
  const signing = await readFile(join(root, 'scripts', 'test-signing-chain.ps1'), 'utf8');
  const invocation = await readFile(join(root, 'scripts', 'run-real-invocation.ps1'), 'utf8');
  const cleanVm = await readFile(join(root, 'scripts', 'verify-clean-vm.ps1'), 'utf8');
  const signingDoc = await readFile(join(root, 'docs', 'release', 'windows-signing.md'), 'utf8');
  assert.match(signing, /TEST-self-signed/);
  assert.match(signing, /certificateType/);
  assert.match(signing, /signtool.*verify|verify.*signtool/i);
  assert.match(signing, /verify-codex-release\.ps1/);
  assert.match(signingDoc, /MISSION_CONTROL_SIGNING_PFX_BASE64/);
  assert.match(signingDoc, /TAURI_SIGNING_PRIVATE_KEY|TAURI_UPDATER_SIGNATURE/);
  assert.match(invocation, /IAuthorizeRealInvocation/);
  assert.match(invocation, /realInvocation\s*=\s*['"]deferred['"]/i);
  assert.match(invocation, /requestApproval|approval/i);
  assert.match(invocation, /restart|resume/i);
  assert.match(invocation, /cost|tokenLimit|durationLimitSeconds/i);
  assert.match(invocation, /codex-task-matrix\.md/);
  assert.match(cleanVm, /blocked/i);
  assert.match(cleanVm, /supervisor\.ready|named pipe|credential|ledger/i);
});

test('Round 6 readiness scripts produce deferred and blocked reports without real inputs', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-round6-readiness-'));
  try {
    const invocationOutput = join(directory, 'invocation');
    const invocationResult = await execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'run-real-invocation.ps1'),
      '-OutputRoot', invocationOutput,
    ]);
    assert.match(invocationResult.stdout, /realInvocation=deferred/i);
    const invocationRun = (await (await import('node:fs/promises')).readdir(invocationOutput))[0];
    const invocationReport = JSON.parse((await readFile(join(invocationOutput, invocationRun, 'real-invocation-report.json'), 'utf8')).replace(/^\uFEFF/, ''));
    assert.equal(invocationReport.realInvocation, 'deferred');
    assert.equal(invocationReport.mode, 'fixture');
    assert.equal(invocationReport.taskCategories.length, 10);
    assert.equal(invocationReport.approvalDefault, 'decline');
    assert.equal(invocationReport.jobObject, 'attached');
    await assert.rejects(
      execFileAsync('powershell.exe', [
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'run-real-invocation.ps1'),
        '-IAuthorizeRealInvocation', '-OutputRoot', join(directory, 'unauthorized'),
      ]),
      /requires CODEX_REAL_INVOCATION_COMMAND/i,
    );

    const vmReport = join(directory, 'clean-vm-report.json');
    await assert.rejects(
      execFileAsync('powershell.exe', [
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'verify-clean-vm.ps1'),
        '-OutputPath', vmReport,
      ]),
      /blocked|clean.?vm/i,
    );
    const cleanVmReport = JSON.parse((await readFile(vmReport, 'utf8')).replace(/^\uFEFF/, ''));
    assert.equal(cleanVmReport.status, 'blocked');
    assert.equal(cleanVmReport.releaseReady, false);
    assert.equal(cleanVmReport.result, 'BLOCKED');
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('real invocation harness records an explicitly narrowed task scope', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-round8-scope-'));
  try {
    const outputRoot = join(directory, 'invocation');
    await execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'run-real-invocation.ps1'),
      '-TaskCategories', 'readonly_explanation', '-OutputRoot', outputRoot,
    ]);
    const invocationRun = (await (await import('node:fs/promises')).readdir(outputRoot))[0];
    const report = JSON.parse((await readFile(join(outputRoot, invocationRun, 'real-invocation-report.json'), 'utf8')).replace(/^\uFEFF/, ''));
    assert.deepEqual(report.taskCategories, ['readonly_explanation']);
    assert.equal(report.restartResumeSample, false);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('real invocation harness preserves the authorized budget basis in dry output and evidence', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-round13-budget-basis-'));
  try {
    const outputRoot = join(directory, 'invocation');
    const runId = 'budget-basis-dry-run';
    const budgetBasis = 'R8 readonly_explanation completed usage 303259 totalTokens x 1.65 margin';
    const result = await execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'run-real-invocation.ps1'),
      '-InvocationTransport', 'app-server', '-TokenLimit', '500000', '-DurationLimitSeconds', '900',
      '-TaskCategories', 'readonly_explanation', '-BudgetBasis', budgetBasis,
      '-OutputRoot', outputRoot, '-RunId', runId,
    ]);
    assert.match(result.stdout, new RegExp(`budgetBasis=${budgetBasis.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`));
    const report = JSON.parse((await readFile(join(outputRoot, runId, 'real-invocation-report.json'), 'utf8')).replace(/^\uFEFF/, ''));
    assert.equal(report.budgetBasis, budgetBasis);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('authorized invocation metadata is concrete or explicitly unavailable', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-invocation-metadata-'));
  try {
    const outputRoot = join(directory, 'invocation');
    const payload = JSON.stringify({ type: 'turn.completed', usage: { input_tokens: 3, cached_input_tokens: 1, output_tokens: 5, reasoning_output_tokens: 2 } });
    const encodedPayload = Buffer.from(payload, 'utf8').toString('base64');
    const command = `node.exe -e "process.stdout.write(Buffer.from('${encodedPayload}','base64').toString())"`;
    await execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'run-real-invocation.ps1'),
      '-IAuthorizeRealInvocation', '-OutputRoot', outputRoot,
    ], { env: { ...process.env, CODEX_REAL_INVOCATION_COMMAND: command, CODEX_REAL_INVOCATION_MODEL: 'test-model' } });
    const invocationRun = (await (await import('node:fs/promises')).readdir(outputRoot))[0];
    const report = JSON.parse((await readFile(join(outputRoot, invocationRun, 'real-invocation-report.json'), 'utf8')).replace(/^\uFEFF/, ''));
    assert.equal(report.realInvocation, 'authorized-real');
    assert.match(String(report.codexVersion), /^(codex-cli\s+\d+\.\d+\.\d+|unavailable)$/);
    assert.equal(report.model, 'test-model');
    assert.deepEqual(report.tokenUsage, {
      status: 'reported',
      inputTokens: 3,
      cachedInputTokens: 1,
      cacheWriteInputTokens: 0,
      outputTokens: 5,
      reasoningOutputTokens: 2,
      totalTokens: 8,
    });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('review bundle retains the authorized invocation evidence references', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-review-bundle-'));
  try {
    const gateRoot = join(directory, 'gate');
    const externalRun = join(directory, 'real-invocation');
    await mkdir(gateRoot);
    await mkdir(externalRun);
    await writeFile(join(externalRun, 'events.jsonl'), '{"type":"turn.completed"}\n', 'utf8');
    await writeFile(join(externalRun, 'real-invocation-report.json'), JSON.stringify({ evidencePath: 'events.jsonl' }), 'utf8');
    const gate = {
      schema: 'mission-control-codex-release-gate-v1',
      signingStatus: 'deferred-ci',
      realInvocation: 'authorized-real',
      evidenceStatus: 'controlled-deferred',
      controlledInternalPreview: true,
      formalReleaseBlockedBy: ['signing-credentials', 'clean-vm'],
      failures: [],
      gates: [],
      round6Verification: {},
      artifacts: { realInvocationReport: join(externalRun, 'real-invocation-report.json') },
    };
    await writeFile(join(gateRoot, 'codex-release-gate.json'), JSON.stringify(gate), 'utf8');
    for (const name of ['crash-matrix.json', 'soak.json', 'task-matrix-report.json', 'preview-report.json', 'sbom.json']) {
      await writeFile(join(gateRoot, name), '{}', 'utf8');
    }
    const matrixPath = join(directory, 'finding-matrix.json');
    const bundlePath = join(directory, 'review-bundle.json');
    await execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'build-p1-finding-matrix.ps1'),
      '-GateRoot', gateRoot, '-MatrixPath', matrixPath, '-BundlePath', bundlePath,
    ]);
    const bundle = JSON.parse((await readFile(bundlePath, 'utf8')).replace(/^\uFEFF/, ''));
    const names = bundle.artifacts.map((artifact) => artifact.name);
    assert.ok(names.includes('real-invocation-report.json'));
    assert.ok(names.includes('events.jsonl'));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('Round 7 closeout keeps hygiene, P3 coverage, and GA runbook contracts explicit', async () => {
  const hygiene = await readFile(join(root, 'docs', 'release', 'machine-hygiene-checklist.md'), 'utf8');
  const runbook = await readFile(join(root, 'docs', 'release', 'formal-release-runbook.md'), 'utf8');
  const setup = await readFile(join(root, 'docs', 'development', 'windows-setup.md'), 'utf8');
  const matrix = await readFile(join(root, 'scripts', 'build-p1-finding-matrix.ps1'), 'utf8');
  const cleanup = await readFile(join(root, 'scripts', 'cleanup-test-certificates.ps1'), 'utf8');
  const releaseGate = await readFile(join(root, 'scripts', 'verify-codex-release.ps1'), 'utf8');
  for (const finding of ['F-025a', 'F-025b', 'F-025c', 'F-026a', 'F-026b', 'F-027', 'F-029']) assert.match(matrix, new RegExp(finding));
  assert.match(matrix, /F-039/);
  assert.match(matrix, /F-040/);
  assert.match(cleanup, /CurrentUser\\My|CurrentUser\\Root/);
  assert.match(cleanup, /thumbprint/i);
  assert.match(hygiene, /certutil|SystemCertificates|post-restart/i);
  assert.match(setup, /Strawberry Perl|Locale\\.Maketext|openssl-sys/i);
  assert.match(matrix, /round8Items/);
  assert.match(matrix, /round9Items/);
  assert.match(matrix, /round12Items/);
  assert.match(releaseGate, /RealInvocationReport/);
  assert.match(releaseGate, /authorized-real/);
  for (const token of ['MISSION_CONTROL_SIGNING_PFX_BASE64', 'run-real-invocation.ps1', 'IAuthorizeRealInvocation', 'verify-clean-vm.ps1', 'releaseReady=false']) assert.match(runbook, new RegExp(token.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')));
});

test('app-server transport aborts immediately when streamed usage exceeds the token limit', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-budget-app-server-'));
  try {
    const outputRoot = join(directory, 'invocation');
    const emitter = join(directory, 'emit-usage.cjs');
    await writeFile(emitter, [
      "process.stdout.write(JSON.stringify({method:'turn/started',params:{threadId:'thread-1',turn:{id:'turn-1'}}})+'\\n');",
      "process.stdout.write(JSON.stringify({method:'thread/tokenUsage/updated',params:{threadId:'thread-1',turnId:'turn-1',tokenUsage:{last:{inputTokens:8,cachedInputTokens:2,outputTokens:1,reasoningOutputTokens:0,totalTokens:9},total:{inputTokens:8,cachedInputTokens:2,outputTokens:1,reasoningOutputTokens:0,totalTokens:9}}}})+'\\n');",
      "setTimeout(()=>process.stdout.write(JSON.stringify({method:'thread/tokenUsage/updated',params:{threadId:'thread-1',turnId:'turn-1',tokenUsage:{last:{inputTokens:16,cachedInputTokens:4,outputTokens:4,reasoningOutputTokens:0,totalTokens:20},total:{inputTokens:16,cachedInputTokens:4,outputTokens:4,reasoningOutputTokens:0,totalTokens:20}}}})+'\\n'),40);",
      "setTimeout(()=>setTimeout(()=>{},10000),80);",
    ].join('\n'), 'utf8');
    const command = `node.exe "${emitter}"`;
    await assert.rejects(execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'run-real-invocation.ps1'),
      '-IAuthorizeRealInvocation', '-SimulationMode', '-InvocationTransport', 'app-server', '-TokenLimit', '10',
      '-DurationLimitSeconds', '5', '-OutputRoot', outputRoot,
    ], { env: { ...process.env, CODEX_REAL_INVOCATION_COMMAND: command, CODEX_REAL_INVOCATION_MODEL: 'test-model' } }), /budget|exceed|exit code/i);
    const invocationRun = (await (await import('node:fs/promises')).readdir(outputRoot))[0];
    const report = JSON.parse((await readFile(join(outputRoot, invocationRun, 'real-invocation-report.json'), 'utf8')).replace(/^\uFEFF/, ''));
    const events = await readFile(join(outputRoot, invocationRun, 'events.jsonl'), 'utf8');
    assert.equal(report.invocationTransport, 'app-server');
    assert.equal(report.mode, 'simulation');
    assert.equal(report.realInvocation, 'deferred');
    assert.match(report.source, /TEST\/DryRun/);
    assert.equal(report.tokenLimit, 10);
    assert.equal(report.budgetExceeded, true);
    assert.equal(report.actualUsage.inputTokens, 16);
    assert.match(events, /turn\/interrupt/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('authorized app-server harness drives the initialize thread and turn handshake', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-app-server-handshake-'));
  try {
    const outputRoot = join(directory, 'invocation');
    const server = join(directory, 'server.cjs');
    await writeFile(server, [
      "const readline=require('node:readline');",
      "const rl=readline.createInterface({input:process.stdin});",
      "const send=(m)=>process.stdout.write(JSON.stringify(m)+'\\n');",
      "rl.on('line',line=>{ const m=JSON.parse(line); if(m.method==='initialize'){ send({jsonrpc:'2.0',id:m.id,result:{}}); } else if(m.method==='thread/start'){ if(m.params.sandbox!=='read-only'){ send({jsonrpc:'2.0',id:m.id,error:{code:-32602,message:'sandbox must be read-only'}}); process.exit(2); } send({jsonrpc:'2.0',id:m.id,result:{thread:{id:'real-thread-1'}}}); send({method:'thread/started',params:{thread:{id:'real-thread-1'}}}); } else if(m.method==='turn/start'){ send({jsonrpc:'2.0',id:m.id,result:{turn:{id:'real-turn-1'}}}); send({method:'turn/started',params:{threadId:'real-thread-1',turn:{id:'real-turn-1'}}}); send({method:'thread/tokenUsage/updated',params:{threadId:'real-thread-1',turnId:'real-turn-1',tokenUsage:{total:{inputTokens:12,cachedInputTokens:2,outputTokens:4,reasoningOutputTokens:0,totalTokens:16}}}}); send({method:'turn/completed',params:{threadId:'real-thread-1',turnId:'real-turn-1'}}); setTimeout(()=>process.exit(0),50); } });",
    ].join('\n'), 'utf8');
    const command = `node.exe "${server}"`;
    await execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'run-real-invocation.ps1'),
      '-IAuthorizeRealInvocation', '-SimulationMode', '-InvocationTransport', 'app-server', '-TaskCategories', 'readonly_explanation',
      '-TokenLimit', '100', '-DurationLimitSeconds', '5', '-OutputRoot', outputRoot,
    ], { env: { ...process.env, CODEX_REAL_INVOCATION_COMMAND: command, CODEX_REAL_INVOCATION_MODEL: 'test-model' } });
    const invocationRun = (await (await import('node:fs/promises')).readdir(outputRoot))[0];
    const report = JSON.parse((await readFile(join(outputRoot, invocationRun, 'real-invocation-report.json'), 'utf8')).replace(/^\uFEFF/, ''));
    const events = await readFile(join(outputRoot, invocationRun, 'events.jsonl'), 'utf8');
    assert.equal(report.invocationTransport, 'app-server');
    assert.equal(report.budgetExceeded, false);
    assert.equal(report.actualUsage.totalTokens, 16);
    assert.match(events, /"method":"initialize"/);
    assert.match(events, /"method":"thread\/start"/);
    assert.match(events, /"method":"turn\/start"/);
    assert.match(events, /thread\/tokenUsage\/updated/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('non-zero app-server exit after turn completion remains a failed deferred run', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-app-server-nonzero-exit-'));
  try {
    const outputRoot = join(directory, 'invocation');
    const command = `node.exe -e "const send=o=>process.stdout.write(JSON.stringify(o)+'\\n'); let stage=0; require('readline').createInterface({input:process.stdin}).on('line',line=>{ const m=JSON.parse(line); if(m.method==='initialize'){ send({jsonrpc:'2.0',id:m.id,result:{}}); } else if(m.method==='thread/start'){ send({jsonrpc:'2.0',id:m.id,result:{thread:{id:'failed-thread'}}}); } else if(m.method==='turn/start'){ send({jsonrpc:'2.0',id:m.id,result:{turn:{id:'failed-turn'}}}); send({method:'turn/started',params:{threadId:'failed-thread',turn:{id:'failed-turn'}}}); send({method:'turn/completed',params:{threadId:'failed-thread',turnId:'failed-turn'}}); setTimeout(()=>process.exit(7),50); } });"`;
    await assert.rejects(execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'run-real-invocation.ps1'),
      '-IAuthorizeRealInvocation', '-InvocationTransport', 'app-server', '-TaskCategories', 'readonly_explanation',
      '-TokenLimit', '100', '-DurationLimitSeconds', '5', '-OutputRoot', outputRoot,
    ], { env: { ...process.env, CODEX_REAL_INVOCATION_COMMAND: command, CODEX_REAL_INVOCATION_MODEL: 'test-model' } }));
    const invocationRun = (await (await import('node:fs/promises')).readdir(outputRoot))[0];
    const report = JSON.parse((await readFile(join(outputRoot, invocationRun, 'real-invocation-report.json'), 'utf8')).replace(/^\uFEFF/, ''));
    assert.equal(report.realInvocation, 'deferred');
    assert.equal(report.budgetExceeded, false);
    assert.match(report.violationReason, /exited 7/i);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('technical app-server failure keeps real invocation deferred without budget violation', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-app-server-error-'));
  try {
    const outputRoot = join(directory, 'invocation');
    const command = `node.exe -e "process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:1,error:{code:-32000,message:'login expired'}})+'\\n')"`;
    await assert.rejects(execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'run-real-invocation.ps1'),
      '-IAuthorizeRealInvocation', '-InvocationTransport', 'app-server', '-TaskCategories', 'readonly_explanation',
      '-TokenLimit', '100', '-DurationLimitSeconds', '2', '-OutputRoot', outputRoot,
    ], { env: { ...process.env, CODEX_REAL_INVOCATION_COMMAND: command, CODEX_REAL_INVOCATION_MODEL: 'test-model' } }));
    const invocationRun = (await (await import('node:fs/promises')).readdir(outputRoot))[0];
    const report = JSON.parse((await readFile(join(outputRoot, invocationRun, 'real-invocation-report.json'), 'utf8')).replace(/^\uFEFF/, ''));
    assert.equal(report.realInvocation, 'deferred');
    assert.equal(report.budgetExceeded, false);
    assert.match(report.violationReason, /protocol error|login expired/i);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('app-server protocol error terminates a still-open provider and preserves the error classification', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-app-server-open-error-'));
  try {
    const outputRoot = join(directory, 'invocation');
    const provider = join(directory, 'provider-error-open.mjs');
    await writeFile(provider, "process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:1,error:{code:-32000,message:'protocol rejected'}})+'\\n'); setTimeout(()=>{}, 10000);\n", 'utf8');
    const startedAt = Date.now();
    await assert.rejects(execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'run-real-invocation.ps1'),
      '-IAuthorizeRealInvocation', '-InvocationTransport', 'app-server', '-TaskCategories', 'readonly_explanation',
      '-TokenLimit', '100', '-DurationLimitSeconds', '5', '-OutputRoot', outputRoot,
    ], { env: { ...process.env, CODEX_REAL_INVOCATION_COMMAND: `node.exe "${provider}"`, CODEX_REAL_INVOCATION_MODEL: 'test-model' } }));
    assert.ok(Date.now() - startedAt < 3000, 'protocol errors must terminate the provider without waiting for the duration limit');
    const invocationRun = (await (await import('node:fs/promises')).readdir(outputRoot))[0];
    const report = JSON.parse((await readFile(join(outputRoot, invocationRun, 'real-invocation-report.json'), 'utf8')).replace(/^\uFEFF/, ''));
    assert.equal(report.realInvocation, 'deferred');
    assert.equal(report.budgetExceeded, false);
    assert.match(report.violationReason, /protocol error.*protocol rejected/i);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('exec transport marks an over-budget completion as a post-hoc violation', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-budget-exec-'));
  try {
    const outputRoot = join(directory, 'invocation');
    const payload = JSON.stringify({ type: 'turn.completed', usage: { input_tokens: 8, cached_input_tokens: 2, output_tokens: 5 } });
    const encodedPayload = Buffer.from(payload, 'utf8').toString('base64');
    const command = `node.exe -e "process.stdout.write(Buffer.from('${encodedPayload}','base64').toString())"`;
    await assert.rejects(execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'run-real-invocation.ps1'),
      '-IAuthorizeRealInvocation', '-SimulationMode', '-InvocationTransport', 'exec', '-TokenLimit', '10',
      '-OutputRoot', outputRoot,
    ], { env: { ...process.env, CODEX_REAL_INVOCATION_COMMAND: command, CODEX_REAL_INVOCATION_MODEL: 'test-model' } }), /post-hoc|budget|exit code/i);
    const invocationRun = (await (await import('node:fs/promises')).readdir(outputRoot))[0];
    const report = JSON.parse((await readFile(join(outputRoot, invocationRun, 'real-invocation-report.json'), 'utf8')).replace(/^\uFEFF/, ''));
    assert.equal(report.invocationTransport, 'exec');
    assert.equal(report.mode, 'simulation');
    assert.equal(report.realInvocation, 'deferred');
    assert.equal(report.budgetExceeded, true);
    assert.equal(report.actualUsage.outputTokens, 5);
    assert.match(report.violationReason, /post-hoc violation \(exec transport cannot abort mid-run\)/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('release evidence contracts require explicit invocation transport and approval guidance', async () => {
  const invocation = await readFile(join(root, 'scripts', 'run-real-invocation.ps1'), 'utf8');
  const verify = await readFile(join(root, 'scripts', 'verify-codex-release.ps1'), 'utf8');
  const runbook = await readFile(join(root, 'docs', 'release', 'formal-release-runbook.md'), 'utf8');
  const registry = JSON.parse(await readFile(join(root, 'docs', 'release', 'evidence-bindings.json'), 'utf8'));
  assert.match(invocation, /InvocationTransport/);
  assert.match(invocation, /actualUsage/);
  assert.match(invocation, /budgetExceeded/);
  assert.match(verify, /invocationTransport/);
  assert.match(verify, /reportedUsageTotal|withinTokenLimit/);
  assert.match(runbook, /approval.*app-server|app-server.*approval/i);
  const historical = registry.bindings.find(({ evidencePath }) => evidencePath.endsWith('/real-invocation-report.json'));
  assert.equal(historical.sha256.toUpperCase(), 'D85DFC66E6DD89A4B0CB9598719869B9BA5792873B7C3A3B16B2FDCD78E49924');
  assert.equal(historical.invocationTransport, 'exec');
  assert.equal(historical.availability, 'local-artifact');
});

test('immutable evidence bindings tolerate absent registered local artifacts in a clean clone', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-binding-clean-clone-'));
  try {
    const bindings = join(directory, 'evidence-bindings.json');
    await writeFile(bindings, JSON.stringify({ schema: 'mission-control-evidence-bindings-v1', bindings: [{ evidencePath: 'artifacts/local-only.json', sha256: 'a'.repeat(64), firstBoundBundle: 'artifacts/bundle.json', availability: 'local-artifact' }] }), 'utf8');
    const { stdout } = await execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'verify-evidence-bindings.ps1'),
      '-BindingsPath', bindings, '-RepositoryRoot', directory,
    ]);
    const report = JSON.parse(stdout.trim());
    assert.equal(report.status, 'pass');
    assert.equal(report.checked, 0);
    assert.deepEqual(report.unavailableLocalArtifacts, ['artifacts/local-only.json']);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('immutable evidence bindings reject a changed file even when the report still parses', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-binding-registry-'));
  try {
    const evidence = join(directory, 'real-invocation-report.json');
    const bindings = join(directory, 'evidence-bindings.json');
    await writeFile(evidence, '{"stable":true}\n', 'utf8');
    const stableHash = createHash('sha256').update(await readFile(evidence)).digest('hex');
    await writeFile(bindings, JSON.stringify({ schema: 'mission-control-evidence-bindings-v1', bindings: [{ evidencePath: 'real-invocation-report.json', sha256: stableHash, firstBoundBundle: 'bundle-a' }] }), 'utf8');
    await writeFile(evidence, '{"stable":false}\n', 'utf8');
    await assert.rejects(execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'verify-evidence-bindings.ps1'),
      '-BindingsPath', bindings, '-RepositoryRoot', directory,
    ]), /hash mismatch/i);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('evidence binding verifier checks every review bundle reference against the cumulative registry', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-binding-bundle-'));
  try {
    const evidence = join(directory, 'events.jsonl');
    const bindings = join(directory, 'evidence-bindings.json');
    const bundle = join(directory, 'review-bundle.json');
    await writeFile(evidence, '{"method":"turn/completed"}\n', 'utf8');
    const stableHash = createHash('sha256').update(await readFile(evidence)).digest('hex');
    await writeFile(bindings, JSON.stringify({ schema: 'mission-control-evidence-bindings-v1', bindings: [{ evidencePath: 'events.jsonl', sha256: stableHash, firstBoundBundle: 'review-bundle.json' }] }), 'utf8');
    await writeFile(bundle, JSON.stringify({ artifacts: [{ name: 'events.jsonl', sha256: stableHash }] }), 'utf8');
    await execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'verify-evidence-bindings.ps1'),
      '-BindingsPath', bindings, '-RepositoryRoot', directory, '-ReviewBundlePath', bundle,
    ]);
    await writeFile(bundle, JSON.stringify({ artifacts: [{ name: 'events.jsonl', sha256: '0'.repeat(64) }] }), 'utf8');
    await assert.rejects(execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'verify-evidence-bindings.ps1'),
      '-BindingsPath', bindings, '-RepositoryRoot', directory, '-ReviewBundlePath', bundle,
    ]), /bundle.*hash|binding.*mismatch/i);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('same-name evidence mismatch fails closed before bundle generation', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'codex-evidence-mismatch-'));
  try {
    const gateRoot = join(directory, 'gate');
    const externalRun = join(directory, 'external');
    await mkdir(gateRoot);
    await mkdir(externalRun);
    await writeFile(join(gateRoot, 'codex-release-gate.json'), JSON.stringify({ schema: 'mission-control-codex-release-gate-v1', artifacts: { realInvocationReport: join(externalRun, 'real-invocation-report.json') }, gates: [], failures: [], controlledInternalPreview: true, evidenceStatus: 'controlled-deferred', formalReleaseBlockedBy: ['signing-credentials', 'clean-vm'] }), 'utf8');
    for (const name of ['crash-matrix.json', 'soak.json', 'task-matrix-report.json', 'preview-report.json', 'sbom.json']) await writeFile(join(gateRoot, name), '{}', 'utf8');
    await writeFile(join(gateRoot, 'real-invocation-report.json'), '{"source":"gate"}', 'utf8');
    await writeFile(join(externalRun, 'real-invocation-report.json'), '{"source":"external"}', 'utf8');
    await assert.rejects(execFileAsync('powershell.exe', [
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts', 'build-p1-finding-matrix.ps1'),
      '-GateRoot', gateRoot, '-MatrixPath', join(directory, 'finding-matrix.json'), '-BundlePath', join(directory, 'review-bundle.json'),
    ]), /same-name evidence mismatch/i);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
