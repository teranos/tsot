import { byteRange } from './src/range'

// main.ts is bundled from disk on every request. Bun's dev watcher holds
// the inode it first opened; editors replace files with a new inode, so
// the watcher kept serving a stale bundle.
const PORT = Number(process.env.PORT ?? 5190)
const js = { 'content-type': 'text/javascript' }

async function bundle(): Promise<Response> {
  try {
    const out = await Bun.build({ entrypoints: [`${import.meta.dir}/src/main.ts`], target: 'browser' })
    return new Response(out.outputs[0], { headers: js })
  } catch (e) {
    const text = e instanceof AggregateError ? e.errors.map(String).join('\n') : String(e)
    console.error(`[build] main.ts failed:\n${text}`)
    const onPage = `const box = document.getElementById('error')
box.textContent = ${JSON.stringify(`build failed: main.ts\n${text}`)}
box.hidden = false
document.documentElement.dataset.booted = 'build-failed'`
    return new Response(onPage, { headers: js })
  }
}

// Bun.serve ignores Range on file responses; video seeks and loops by range.
function video(name: string) {
  const path = `${import.meta.dir}/assets/${name}`
  return async (req: Request): Promise<Response> => {
    const file = Bun.file(path)
    if (!(await file.exists())) {
      console.error(`[video] missing: ${path}`)
      return new Response(`missing: ${path}`, { status: 404 })
    }
    const size = file.size
    const base = { 'accept-ranges': 'bytes', 'content-type': 'video/mp4' }
    let r: ReturnType<typeof byteRange>
    try {
      r = byteRange(req.headers.get('range'), size)
    } catch (e) {
      console.error(`[video] ${name}: ${e}`)
      return new Response(String(e), { status: 416, headers: { 'content-range': `bytes */${size}` } })
    }
    if (!r) return new Response(file, { headers: { ...base, 'content-length': String(size) } })
    return new Response(file.slice(r.start, r.end + 1), {
      status: 206,
      headers: {
        ...base,
        'content-range': `bytes ${r.start}-${r.end}/${size}`,
        'content-length': String(r.end - r.start + 1),
      },
    })
  }
}

function asset(path: string) {
  const full = `${import.meta.dir}/assets/${path}`
  return async (): Promise<Response> => {
    const file = Bun.file(full)
    if (!(await file.exists())) {
      console.error(`[asset] missing: ${full}`)
      return new Response(`missing: ${full}`, { status: 404 })
    }
    return new Response(file)
  }
}

const server = Bun.serve({
  port: PORT,
  routes: {
    '/': () => new Response(Bun.file(`${import.meta.dir}/index.html`)),
    '/main.js': bundle,
    '/hell.mp4': video('hell.mp4'),
    '/scale/scale.obj': asset('scale/scale.obj'),
    '/scale/diffuse.jpg': asset('scale/diffuse.jpg'),
    '/scale/normal.jpg': asset('scale/normal.jpg'),
  },
})

console.log(`psychostasia: ${server.url}`)
