import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import { prepareSidecar } from '../prepare-sidecar.mjs';

test('prepareSidecar writes the stable ICO and copies the release sidecar', async () => {
  const root = await mkdtemp(join(tmpdir(), 'mission-prepare-sidecar-'));
  try {
    const source = join(root, 'release', 'mission-control-supervisor.exe');
    const sidecar = join(root, 'binaries', 'mission-control-supervisor-x86_64-pc-windows-msvc.exe');
    const icon = join(root, 'generated', 'mission-control.ico');
    await mkdir(join(root, 'release'));
    await writeFile(source, Buffer.from('MZ-fixture'));

    await prepareSidecar({ source, sidecar, icon });

    assert.deepEqual(await readFile(sidecar), Buffer.from('MZ-fixture'));
    const bytes = await readFile(icon);
    assert.equal(bytes.length, 70);
    assert.deepEqual([...bytes.subarray(0, 6)], [0, 0, 1, 0, 1, 0]);
    assert.equal(
      createHash('sha256').update(bytes).digest('hex'),
      '0a61fae93e57023c8492d0dc7c4c4b64ea5b72511edfbce83c610c6ee5f64b41',
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
