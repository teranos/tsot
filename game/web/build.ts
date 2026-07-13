// game.sbvh.nl bundle builder. Bundles src/main.ts + copies index.html
// into dist/. No Svelte, no runtime deps — plain wasm+JS shim.

import { join, basename } from 'path'
import { rm, mkdir, copyFile, cp, stat } from 'fs/promises'

const srcDir = join(import.meta.dir, 'src')
const outDir = join(import.meta.dir, 'dist')

await rm(outDir, { recursive: true, force: true }).catch(() => {})
await mkdir(outDir, { recursive: true })

// Content-hash the bundle filename. index.html is served no-store (always
// fresh) but references this file; without a content hash it points at a
// mutable `/main.js` that a browser can cache (max-age) and then pair a
// STALE bundle with a newer game.wasm. When the wasm gains an env.*
// import (e.g. game_gpu_render_glass) the old bundle can't provide it and
// boot dies with "import function ... must be callable". A hash in the
// name makes the pairing atomic: a new bundle is a new URL, and the
// fresh index.html can only ever reference the matching one.
const result = await Bun.build({
  entrypoints: [join(srcDir, 'main.ts')],
  outdir: outDir,
  minify: false,
  sourcemap: 'inline',
  naming: '[name]-[hash].[ext]',
})

if (!result.success) {
  console.error('Build failed:')
  for (const msg of result.logs) console.error(msg)
  process.exit(1)
}

const entry = result.outputs.find(o => o.kind === 'entry-point')
if (!entry) {
  console.error('Build produced no entry-point output')
  process.exit(1)
}
const bundleName = basename(entry.path) // main-<hash>.js

await copyFile(join(import.meta.dir, 'style.css'), join(outDir, 'style.css'))

// Copy assets/ recursively if it exists. Missing dir is not an error —
// the audio module's load path fails silently when the URL 404s.
const assetsSrc = join(import.meta.dir, 'assets')
const assetsExists = await stat(assetsSrc).then(s => s.isDirectory()).catch(() => false)
if (assetsExists) {
  await cp(assetsSrc, join(outDir, 'assets'), { recursive: true })
}

const html = await Bun.file(join(import.meta.dir, 'index.html')).text()
await Bun.write(join(outDir, 'index.html'), html.replace('/src/main.ts', `/${bundleName}`))

console.log(`game web bundle built -> dist/ (${bundleName})`)
