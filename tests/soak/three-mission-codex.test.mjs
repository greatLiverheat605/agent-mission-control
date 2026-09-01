import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createInterface } from 'node:readline';
import { join, resolve } from 'node:path';
import { spawn } from 'node:child_process';
import { test } from 'node:test';

const root = resolve(import.meta.dirname, '../..');
const fixture = join(root, 'fixtures', 'agents', 'bin', 'fake-codex-app-server.ps1');

function runFixture(missionId) {
  return new Promise((resolveMission, reject) => {
    const child = spawn('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', fixture], { cwd: root, env: { ...process.env, FAKE_CODEX_MISSION_ID: missionId }, stdio: ['pipe', 'pipe', 'pipe'] });
    const lines = createInterface({ input: child.stdout });
    const pending = [];
    const events = [];
    lines.on('line', (line) => {
      try {
        const message = JSON.parse(line);
        events.push(message);
        for (let index = pending.length - 1; index >= 0; index -= 1) {
          if (pending[index].predicate(message)) {
            const waiter = pending.splice(index, 1)[0];
            waiter.resolve(message);
          }
        }
      } catch (error) {
        reject(error);
      }
    });
    child.on('error', reject);
    const waitFor = (predicate) => new Promise((resolveWait, rejectWait) => {
      const buffered = events.find(predicate);
      if (buffered) { resolveWait(buffered); return; }
      const timer = setTimeout(() => rejectWait(new Error(`fixture timeout for ${missionId}`)), 3000);
      pending.push({ predicate, resolve: (value) => { clearTimeout(timer); resolveWait(value); } });
    });
    const send = (message) => child.stdin.write(`${JSON.stringify(message)}\n`);
    (async () => {
      send({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { clientInfo: { name: 'agent-mission-control', title: 'Agent Mission Control', version: '0.1.0' }, capabilities: {} } });
      await waitFor((message) => message.id === 1 && message.result);
      send({ jsonrpc: '2.0', id: 2, method: 'thread/start', params: { cwd: root, model: null, sandbox: 'read-only' } });
      const threadResponse = await waitFor((message) => message.id === 2 && message.result?.thread?.id);
      const threadId = threadResponse.result.thread.id;
      send({ jsonrpc: '2.0', id: 3, method: 'turn/start', params: { threadId, input: [{ type: 'text', text: `mission goal ${missionId}` }] } });
      await waitFor((message) => message.id === 3 && message.result?.turn?.id);
      await waitFor((message) => message.id === 900 && message.method === 'item/commandExecution/requestApproval');
      send({ jsonrpc: '2.0', id: 900, result: { decision: 'accept' } });
      await waitFor((message) => message.method === 'turn/completed');
      const methods = new Set(events.filter((message) => message.method).map((message) => message.method));
      assert.ok(methods.has('thread/started'));
      assert.ok(methods.has('turn/started'));
      assert.ok(methods.has('item/agentMessage/delta'));
      assert.ok(methods.has('turn/diff/updated'));
      assert.ok(methods.has('thread/tokenUsage/updated'));
      assert.ok(methods.has('item/commandExecution/requestApproval'));
      child.kill();
      resolveMission({ missionId, threadId, realInvocation: 'deferred', approvalDecision: 'accept' });
    })().catch((error) => { child.kill(); reject(error); });
  });
}

test('controlled Codex soak drives three isolated fake app-server missions', async (t) => {
  if (process.platform !== 'win32') t.skip('PowerShell fixture is Windows-only');
  const missions = await Promise.all([1, 2, 3].map((missionId) => runFixture(`mission-${missionId}`)));
  assert.equal(new Set(missions.map((mission) => mission.missionId)).size, 3);
  assert.equal(new Set(missions.map((mission) => mission.threadId)).size, 3);
  assert.ok(missions.every((mission) => mission.realInvocation === 'deferred'));
  assert.ok(missions.every((mission) => mission.approvalDecision === 'accept'));
});
