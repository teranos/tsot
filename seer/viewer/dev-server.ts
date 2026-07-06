#!/usr/bin/env bun
/**
 * seer/viewer dev server
 *
 * Serves the built frontend from dist/ with live reload. For data
 * requests (/history.json, /<sha>/<file>) it either:
 *   - serves from a local seer-host output dir when SEER_DEV_DATA
 *     is set (SEER_DEV_DATA + pathname is read from disk), OR
 *   - proxies through to SEER_PROD_ORIGIN (default seer.sbvh.nl)
 *     so the viewer devs against real production history without
 *     re-running seer-host locally.
 *
 * No hardcoded API contract with a Rust binary — the entire data
 * layer is flat JSON at S3 (or a mirror). This dev-server is just
 * pipe + live-reload.
 */

import { watch } from 'fs'
import { exec } from 'child_process'
import { promisify } from 'util'
import { join } from 'path'

const execAsync = promisify(exec)

const DEV_PORT = Number(process.env.SEER_DEV_PORT || 5180)
// Frontend fetches /perf/<sha>/<file> directly (see lib/fetch.ts
// DATA_BASE constant). Dev-server just forwards the same pathname
// to the proxy origin, no rewriting.
const PROD_ORIGIN = process.env.SEER_PROD_ORIGIN || 'https://seer.sbvh.nl'
const DATA_DIR = process.env.SEER_DEV_DATA || ''
const distDir = join(import.meta.dir, 'dist')

const globalFetch = globalThis.fetch

let isBuilding = false
let buildTimeout: ReturnType<typeof setTimeout> | null = null
const reloadClients: Set<{ write: (data: string) => void }> = new Set()

async function build() {
  if (isBuilding) return
  isBuilding = true
  console.log('Building...')
  try {
    await execAsync('bun run build.ts', { cwd: import.meta.dir })
    console.log('Build complete')
    reloadClients.forEach(c => c.write('data: reload\n\n'))
  } catch (e: any) {
    console.error('Build failed:', e.stderr || e.message)
  } finally {
    isBuilding = false
  }
}

watch(join(import.meta.dir, 'src'), { recursive: true }, (_event, filename) => {
  if (!filename) return
  console.log(`Changed: ${filename}`)
  if (buildTimeout) clearTimeout(buildTimeout)
  buildTimeout = setTimeout(build, 300)
})

watch(join(import.meta.dir, 'index.html'), () => {
  if (buildTimeout) clearTimeout(buildTimeout)
  buildTimeout = setTimeout(build, 300)
})

await build()

// All data artifacts live under /perf/. If a request matches that
// prefix and it isn't a locally-served static file, we route it to
// the local mirror (SEER_DEV_DATA) or proxy through to production.
function isDataPath(pathname: string): boolean {
  return pathname.startsWith('/perf/')
}

async function serveData(pathname: string): Promise<Response> {
  if (DATA_DIR) {
    const localPath = join(DATA_DIR, pathname)
    const file = Bun.file(localPath)
    if (await file.exists()) return new Response(file)
    return new Response(`local not found: ${localPath}`, { status: 404 })
  }
  try {
    const upstream = await globalFetch(`${PROD_ORIGIN}${pathname}`)
    return new Response(upstream.body, {
      status: upstream.status,
      headers: {
        'content-type': upstream.headers.get('content-type') || 'application/octet-stream',
      },
    })
  } catch (e: any) {
    return new Response(`proxy failed: ${e?.message || e}`, { status: 502 })
  }
}

const server = Bun.serve({
  port: DEV_PORT,
  idleTimeout: 60,
  async fetch(req) {
    const url = new URL(req.url)

    if (url.pathname === '/__dev_reload__') {
      return new Response(
        new ReadableStream({
          start(controller) {
            const client = {
              write: (data: string) => controller.enqueue(new TextEncoder().encode(data)),
            }
            reloadClients.add(client)
            req.signal.addEventListener('abort', () => reloadClients.delete(client))
          },
        }),
        { headers: { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache' } },
      )
    }

    // Static files from dist/ take precedence over data-shaped paths so
    // that /main.js, /tokens.css, /font.css never accidentally route to
    // the proxy.
    if (url.pathname !== '/' && url.pathname !== '') {
      const file = Bun.file(join(distDir, url.pathname))
      if (await file.exists()) return new Response(file)
    }

    if (isDataPath(url.pathname)) {
      return serveData(url.pathname)
    }

    // Root or unmatched → serve index.html with live-reload injected
    let html = await Bun.file(join(distDir, 'index.html')).text()
    html = html.replace(
      '</body>',
      `<script>
        const es = new EventSource("/__dev_reload__");
        es.onmessage = () => location.reload();
      </script></body>`,
    )
    return new Response(html, { headers: { 'Content-Type': 'text/html' } })
  },
})

console.log(`seer/viewer dev: http://localhost:${server.port}`)
if (DATA_DIR) {
  console.log(`data: local ${DATA_DIR}`)
} else {
  console.log(`data: proxying ${PROD_ORIGIN}`)
}
