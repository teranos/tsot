/**
 * seer viewer builder
 *
 * Compiles Svelte 5 via Bun.build() plugin.
 * Outputs bundled JS + index.html + tokens.css + font.css to dist/.
 */

import { compile } from 'svelte/compiler'
import { join } from 'path'
import { rm, mkdir, copyFile } from 'fs/promises'

const srcDir = join(import.meta.dir, 'src')
const outDir = join(import.meta.dir, 'dist')

const sveltePlugin: import('bun').BunPlugin = {
  name: 'svelte',
  setup(build) {
    build.onLoad({ filter: /\.svelte$/ }, async (args) => {
      const source = await Bun.file(args.path).text()
      const result = compile(source, {
        filename: args.path,
        generate: 'client',
      })
      let code = result.js.code
      if (result.css && result.css.code) {
        const escaped = result.css.code
          .replaceAll('\\', '\\\\')
          .replaceAll('`', '\\`')
          .replaceAll('$', '\\$')
        code += `\n;(function(){const s=document.createElement('style');s.textContent=\`${escaped}\`;document.head.appendChild(s)})()\n`
      }
      return { contents: code, loader: 'js' }
    })
  },
}

await rm(outDir, { recursive: true, force: true }).catch(() => {})
await mkdir(outDir, { recursive: true })

const result = await Bun.build({
  entrypoints: [join(srcDir, 'main.ts')],
  outdir: outDir,
  minify: false,
  sourcemap: 'inline',
  plugins: [sveltePlugin],
})

if (!result.success) {
  console.error('Build failed:')
  for (const msg of result.logs) console.error(msg)
  process.exit(1)
}

await copyFile(join(srcDir, 'tokens.css'), join(outDir, 'tokens.css'))
await copyFile(join(srcDir, 'font.css'), join(outDir, 'font.css'))

const html = await Bun.file(join(import.meta.dir, 'index.html')).text()
await Bun.write(join(outDir, 'index.html'), html.replace('/src/main.ts', '/main.js'))

console.log('seer viewer built -> dist/')
