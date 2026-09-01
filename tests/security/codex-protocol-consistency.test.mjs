import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { test } from 'node:test';

const root = resolve(import.meta.dirname, '../..');
const vendoredSchemaRoot = join(root, 'fixtures/protocol/codex-schema-0.147.0');
const schemaRoot = process.env.CODEX_SCHEMA_ROOT ?? vendoredSchemaRoot;

async function schemaFiles(directory, prefix = '') {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name))) {
    const relative = join(prefix, entry.name).replaceAll('\\', '/');
    if (entry.isDirectory()) files.push(...await schemaFiles(join(directory, entry.name), relative));
    else if (entry.isFile()) files.push(relative);
  }
  return files;
}

async function sha256(path) {
  return createHash('sha256').update(await readFile(path)).digest('hex');
}

async function assertVendoredSnapshotIsIntact() {
  const manifest = JSON.parse(await readFile(join(vendoredSchemaRoot, 'snapshot-manifest.json'), 'utf8'));
  const files = await schemaFiles(vendoredSchemaRoot);
  const actual = Object.fromEntries(await Promise.all(files.filter((file) => file.endsWith('.json') && file !== 'snapshot-manifest.json').map(async (file) => [file, await sha256(join(vendoredSchemaRoot, file))])));
  assert.deepEqual(actual, manifest.files, 'vendored Codex schema file hash drift');
  const aggregate = createHash('sha256').update(JSON.stringify(actual)).digest('hex');
  assert.equal(aggregate, manifest.aggregateSha256, 'vendored Codex schema aggregate hash drift');
  if (process.env.CODEX_SCHEMA_ROOT) {
    const overrideFiles = (await schemaFiles(schemaRoot)).filter((file) => file.endsWith('.json')).sort();
    assert.deepEqual(overrideFiles, Object.keys(actual).sort(), 'schema override file set differs from vendored snapshot');
    for (const file of overrideFiles) assert.equal(await sha256(join(schemaRoot, file)), actual[file], `schema override drift: ${file}`);
  }
}

async function schemaMethods(file) {
  const source = await readFile(join(schemaRoot, file), 'utf8');
  return new Set([...source.matchAll(/"method"\s*:\s*\{\s*"enum"\s*:\s*\[\s*"([^"]+)"/g)].map((match) => match[1]));
}

async function readSchema(file) {
  return JSON.parse(await readFile(join(schemaRoot, file), 'utf8'));
}

function requestWindows(source, method) {
  const escaped = method.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const marker = new RegExp(`["']${escaped}["']`, 'g');
  const windows = [...source.matchAll(marker)]
    .filter((match) => /(?:\.request\s*\(|Send-(?:AppServer)?Request\s*\()/i.test(source.slice(Math.max(0, match.index - 160), match.index)))
    .map((match) => source.slice(match.index, match.index + 600));
  assert.ok(windows.length > 0, `request method is not sent: ${method}`);
  return windows;
}

async function assertParameterContracts({ processSource, harnessSource }) {
  const [initialize, threadStart, turnStart, turnInterrupt] = await Promise.all([
    readSchema('v1/InitializeParams.json'),
    readSchema('v2/ThreadStartParams.json'),
    readSchema('v2/TurnStartParams.json'),
    readSchema('v2/TurnInterruptParams.json'),
  ]);
  const sandboxEnum = threadStart.definitions.SandboxMode.enum;
  assert.deepEqual(sandboxEnum, ['read-only', 'workspace-write', 'danger-full-access']);
  assert.deepEqual(initialize.required, ['clientInfo']);
  assert.deepEqual(initialize.definitions.ClientInfo.required, ['name', 'version']);
  assert.deepEqual(turnStart.required, ['input', 'threadId']);
  assert.deepEqual(turnInterrupt.required, ['threadId', 'turnId']);

  const initializeFields = /clientInfo[\s\S]{0,220}(?:name)[\s\S]{0,220}(?:title)[\s\S]{0,220}(?:version)/;
  for (const window of requestWindows(processSource, 'initialize')) assert.match(window, initializeFields, 'adapter initialize.clientInfo is incomplete');
  for (const window of requestWindows(harnessSource, 'initialize')) assert.match(window, initializeFields, 'harness initialize.clientInfo is incomplete');

  for (const [source, label] of [[processSource, 'adapter'], [harnessSource, 'harness']]) {
    const threadWindows = requestWindows(source, 'thread/start');
    for (const threadWindow of threadWindows) {
      assert.match(threadWindow, /["']?sandbox["']?\s*[:=]/, `${label} thread/start sandbox is missing`);
      assert.doesNotMatch(threadWindow, /["']?sandbox["']?\s*[:=]\s*\{[\s\S]{0,80}["']?type["']?\s*[:=]\s*["']readOnly["']/i, `${label} thread/start sandbox must be a SandboxMode string enum`);
      const quotedSandboxValues = [...threadWindow.matchAll(/(?:read-only|workspace-write|danger-full-access|readOnly|dangerFullAccess|workspaceWrite)/g)].map((match) => match[0]);
      assert.ok(quotedSandboxValues.length > 0, `${label} thread/start sandbox literal is missing`);
      for (const value of quotedSandboxValues) assert.ok(sandboxEnum.includes(value), `${label} thread/start sandbox value is outside schema: ${value}`);
    }
  }

  for (const [source, label] of [[processSource, 'adapter'], [harnessSource, 'harness']]) {
    for (const turnWindow of requestWindows(source, 'turn/start')) {
      assert.match(turnWindow, /["']?threadId["']?\s*[:=]/, `${label} turn/start.threadId is missing`);
      assert.match(turnWindow, /["']?input["']?\s*[:=]\s*(?:@\(|\[|json!\(\s*\[)/, `${label} turn/start.input must be an array`);
      assert.match(turnWindow, /["']?type["']?\s*[:=]\s*["']text["']/i, `${label} turn/start input element type must be text`);
      assert.match(turnWindow, /["']?text["']?\s*[:=]/, `${label} turn/start input element text is missing`);
    }
  }

  for (const interruptWindow of requestWindows(processSource, 'turn/interrupt')) {
    assert.match(interruptWindow, /["']?threadId["']?\s*[:=]/, 'adapter turn/interrupt.threadId is missing');
    assert.match(interruptWindow, /["']?turnId["']?\s*[:=]/, 'adapter turn/interrupt.turnId is missing');
  }
}

function protocolLiterals(source) {
  return new Set([...source.matchAll(/["']([A-Za-z][A-Za-z0-9]*(?:\/[A-Za-z0-9]+)+)["']/g)].map((match) => match[1]));
}

test('Codex adapter methods and events match the official 0.147.0 schema', async () => {
  await assertVendoredSnapshotIsIntact();
  const [processSource, normalizerSource, fixtureSource] = await Promise.all([
    readFile(join(root, 'crates/adapter-codex/src/process.rs'), 'utf8'),
    readFile(join(root, 'crates/adapter-codex/src/normalize.rs'), 'utf8'),
    readFile(join(root, 'fixtures/agents/bin/fake-codex-app-server.ps1'), 'utf8'),
  ]);
  const official = new Set();
  for (const file of ['ClientRequest.json', 'ServerRequest.json', 'ServerNotification.json']) {
    for (const method of await schemaMethods(file)) official.add(method);
  }

  const usedMethods = new Set([...processSource.matchAll(/\.request\(\s*"([^"]+)"/g)].map((match) => match[1]));
  const approvalFunction = processSource.match(/fn is_approval_method[\s\S]*?\n\}/)?.[0] ?? '';
  for (const method of approvalFunction.matchAll(/"([^"]+)"/g)) usedMethods.add(method[1]);
  for (const method of usedMethods) assert.ok(official.has(method), `method is not in official schema: ${method}`);
  for (const method of ['initialize', 'thread/start', 'thread/resume', 'turn/start', 'turn/interrupt']) {
    assert.ok(usedMethods.has(method), `required method is not sent: ${method}`);
  }

  const eventNames = [
    'thread/started', 'turn/started', 'turn/completed', 'item/started', 'item/completed',
    'item/agentMessage/delta', 'turn/diff/updated', 'thread/tokenUsage/updated', 'error', 'warning',
  ];
  const referencedEvents = new Set([
    ...protocolLiterals(normalizerSource),
    ...protocolLiterals(fixtureSource),
  ]);
  for (const event of referencedEvents) assert.ok(official.has(event), `event or method is not in official schema: ${event}`);
  for (const event of eventNames) {
    assert.match(normalizerSource, new RegExp(`"${event.replaceAll('/', '\\/')}"`), `normalizer missing ${event}`);
    assert.match(fixtureSource, new RegExp(`'${event.replaceAll('/', '\\/')}'`), `fixture missing ${event}`);
    assert.ok(official.has(event), `event is not in official schema: ${event}`);
  }

  for (const oldDialect of ['thread.started', 'turn.started', 'turn.completed', 'item.started', 'approval.requested']) {
    assert.doesNotMatch(normalizerSource, new RegExp(`"${oldDialect.replaceAll('.', '\\.')}`), `old event dialect remains: ${oldDialect}`);
  }
});

test('Codex request parameters conform to vendored schema contracts', async () => {
  const [processSource, harnessSource] = await Promise.all([
    readFile(join(root, 'crates/adapter-codex/src/process.rs'), 'utf8'),
    readFile(join(root, 'scripts/run-real-invocation.ps1'), 'utf8'),
  ]);
  assert.equal(requestWindows(harnessSource, 'thread/start').length, 2, 'harness fixture and authorized thread/start paths must both be checked');
  await assertParameterContracts({ processSource, harnessSource });
});

test('parameter contract gate rejects the Round 10 object sandbox dialect', async () => {
  const processSource = await readFile(join(root, 'crates/adapter-codex/src/process.rs'), 'utf8');
  const harnessSource = await readFile(join(root, 'scripts/run-real-invocation.ps1'), 'utf8');
  const broken = processSource.replace('"sandbox":if read_only {"read-only"} else {"workspace-write"}', '"sandbox":{"type":"readOnly"}');
  assert.notEqual(broken, processSource, 'regression fixture must mutate a real request literal');
  await assert.rejects(
    () => assertParameterContracts({ processSource: broken, harnessSource }),
    /thread\/start sandbox must be a SandboxMode string enum/,
  );
});

function fixtureRequest(child, message) {
  child.stdin.write(`${JSON.stringify(message)}\n`);
}

test('fake app-server rejects invalid sandbox shape and turn input shape', async (t) => {
  if (process.platform !== 'win32') { t.skip('PowerShell fixture is Windows-only'); return; }
  const fixturePath = join(root, 'fixtures', 'agents', 'bin', 'fake-codex-app-server.ps1');
  const child = spawn('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', fixturePath], { cwd: root, stdio: ['pipe', 'pipe', 'pipe'] });
  const lines = createInterface({ input: child.stdout });
  const events = [];
  const waiters = [];
  lines.on('line', (line) => {
    try {
      const message = JSON.parse(line);
      events.push(message);
      for (let index = waiters.length - 1; index >= 0; index -= 1) {
        if (waiters[index].predicate(message)) {
          const waiter = waiters.splice(index, 1)[0];
          clearTimeout(waiter.timer);
          waiter.resolve(message);
        }
      }
    } catch { /* malformed fixture output is handled by the timeout */ }
  });
  const waitFor = (predicate) => new Promise((resolveMessage, rejectMessage) => {
    const timer = setTimeout(() => rejectMessage(new Error('fixture response timeout')), 3000);
    const buffered = events.find(predicate);
    if (buffered) { clearTimeout(timer); resolveMessage(buffered); return; }
    waiters.push({ predicate, resolve: resolveMessage, timer });
  });
  try {
    fixtureRequest(child, { jsonrpc: '2.0', id: 1, method: 'initialize', params: { clientInfo: { name: 'test', title: 'test', version: '0.1.0' }, capabilities: {} } });
    assert.equal((await waitFor((m) => m.id === 1)).result !== undefined, true);
    fixtureRequest(child, { jsonrpc: '2.0', id: 2, method: 'thread/start', params: { cwd: root, model: null, sandbox: { type: 'readOnly' } } });
    const sandboxError = await waitFor((m) => m.id === 2);
    assert.equal(sandboxError.error?.code, -32602);
    fixtureRequest(child, { jsonrpc: '2.0', id: 3, method: 'thread/start', params: { cwd: root, model: null, sandbox: 'readOnly' } });
    const enumError = await waitFor((m) => m.id === 3);
    assert.equal(enumError.error?.code, -32602);
    fixtureRequest(child, { jsonrpc: '2.0', id: 4, method: 'turn/start', params: { threadId: 'thread', input: { type: 'text', text: 'bad' } } });
    const inputError = await waitFor((m) => m.id === 4);
    assert.equal(inputError.error?.code, -32602);
  } finally {
    lines.close();
    child.kill();
  }
});
