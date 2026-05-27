#!/usr/bin/env bun
import { rm, mkdir, copyFile } from 'fs/promises';
import { join } from 'path';

declare const Bun: {
  build: (opts: unknown) => Promise<{ success: boolean; logs: unknown[] }>;
  file: (path: string) => { text: () => Promise<string> };
  write: (path: string, data: string) => Promise<number>;
};

const root = import.meta.dir;
const srcDir = join(root, 'src');
const outDir = join(root, 'dist');

await rm(outDir, { recursive: true, force: true }).catch(() => {});
await mkdir(outDir, { recursive: true });

const result = await Bun.build({
  entrypoints: [join(srcDir, 'main.ts')],
  outdir: outDir,
  target: 'browser',
  minify: false,
  sourcemap: 'inline',
});

if (!result.success) {
  console.error('Build failed:');
  for (const msg of result.logs) console.error(msg);
  process.exit(1);
}

await copyFile(join(srcDir, 'styles.css'), join(outDir, 'styles.css'));

const html = await Bun.file(join(root, 'index.html')).text();
await Bun.write(
  join(outDir, 'index.html'),
  html.replace('/src/main.ts', '/main.js'),
);

console.log('tsot built -> dist/');
