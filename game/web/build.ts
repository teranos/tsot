// game.sbvh.nl bundle builder. Bundles src/main.ts + copies index.html
// into dist/. No Svelte, no runtime deps — plain wasm+JS shim.

import { join } from 'path'
import { rm, mkdir } from 'fs/promises'

const srcDir = join(import.meta.dir, 'src')
const outDir = join(import.meta.dir, 'dist')

await rm(outDir, { recursive: true, force: true }).catch(() => {})
await mkdir(outDir, { recursive: true })

const result = await Bun.build({
  entrypoints: [join(srcDir, 'main.ts')],
  outdir: outDir,
  minify: false,
  sourcemap: 'inline',
})

if (!result.success) {
  console.error('Build failed:')
  for (const msg of result.logs) console.error(msg)
  process.exit(1)
}

const html = await Bun.file(join(import.meta.dir, 'index.html')).text()
await Bun.write(join(outDir, 'index.html'), html.replace('/src/main.ts', '/main.js'))

console.log('game web bundle built -> dist/')
