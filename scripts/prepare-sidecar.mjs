import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const PLACEHOLDER_ICON = Buffer.from(
  'AAABAAEAAQEAAAEAIAAwAAAAFgAAACgAAAABAAAAAgAAAAEAIAAAAAAABAAAAAAAAAAAAAAAAAAAAAAAAAByfRT/AAAAAA==',
  'base64',
);

export async function prepareSidecar({ source, sidecar, icon }) {
  const supervisor = await readFile(source);
  await Promise.all([
    mkdir(dirname(sidecar), { recursive: true }),
    mkdir(dirname(icon), { recursive: true }),
  ]);
  await Promise.all([
    writeFile(sidecar, supervisor),
    writeFile(icon, PLACEHOLDER_ICON),
  ]);
}

const scriptPath = fileURLToPath(import.meta.url);
if (process.argv[1] && resolve(process.argv[1]) === scriptPath) {
  const root = resolve(dirname(scriptPath), '..');
  await prepareSidecar({
    source: resolve(root, 'target/release/mission-control-supervisor.exe'),
    sidecar: resolve(
      root,
      'apps/desktop/src-tauri/binaries/mission-control-supervisor-x86_64-pc-windows-msvc.exe',
    ),
    icon: resolve(root, 'apps/desktop/src-tauri/generated/mission-control.ico'),
  });
}
