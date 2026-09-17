// SPDX-License-Identifier: GPL-3.0-or-later

// Fetch Microsoft's build input locally; executable files are never committed.
import { createHash } from 'node:crypto';
import { mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const needed = process.platform === 'win32'
  || process.env.CARGO_BUILD_TARGET?.includes('-windows-')
  || process.argv.includes('--force');
if (!needed) process.exit(0);

const source = 'https://msedge.sf.dl.delivery.mp.microsoft.com/filestreamingservice/files/ae30a660-6c9f-4f57-873b-30350e750fa0/MicrosoftEdgeWebview2Setup.exe';
const expected = '83004a28553bcf2f932bf03564fbab407b8e1f59cd265f8dc99cc53d028e459c';
const directory = new URL('../crates/openlaser/assets/', import.meta.url);
const destination = new URL('MicrosoftEdgeWebview2Setup.exe', directory);
const digest = bytes => createHash('sha256').update(bytes).digest('hex');

let existing;
try {
  existing = await readFile(destination);
} catch (error) {
  if (error.code !== 'ENOENT') throw error;
}
if (existing && digest(existing) === expected) {
  console.log('WebView2 bootstrapper already verified.');
  process.exit(0);
}

console.log('Downloading the WebView2 bootstrapper from Microsoft…');
const response = await fetch(source, { signal: AbortSignal.timeout(30_000) });
if (!response.ok) throw new Error(`Microsoft download failed: HTTP ${response.status}`);
const bytes = Buffer.from(await response.arrayBuffer());
if (digest(bytes) !== expected) {
  throw new Error('WebView2 bootstrapper checksum changed. Review the Microsoft download and update the pinned hash before building.');
}
await mkdir(directory, { recursive: true });
const temporary = `${fileURLToPath(destination)}.${process.pid}.download`;
try {
  await writeFile(temporary, bytes, { flag: 'wx' });
  await rename(temporary, destination);
} finally {
  await rm(temporary, { force: true });
}
console.log('WebView2 bootstrapper downloaded and SHA-256 verified.');
