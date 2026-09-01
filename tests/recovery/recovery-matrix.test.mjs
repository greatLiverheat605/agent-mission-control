import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { test } from 'node:test';

const root = new URL('../..', import.meta.url).pathname.replace(/^\/+([A-Za-z]):/, '$1:');

test('crash matrix names every recovery boundary and never enables auto resume', async () => {
  const script = await readFile(join(root, 'scripts', 'run-crash-matrix.ps1'), 'utf8');
  for (const point of ['before_append', 'inside_transaction', 'after_checkpoint', 'ui_lease_lost', 'key_unavailable']) {
    assert.match(script, new RegExp(point));
  }
  assert.match(script, /auto_resume\s*=\s*\$false/);
  assert.match(script, /data_overwritten\s*=\s*\$false/);
});
