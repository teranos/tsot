#!/usr/bin/env bun
import { watch } from 'fs';
import { exec } from 'child_process';
import { promisify } from 'util';
import { join } from 'path';

declare const Bun: {
  serve: (opts: unknown) => { port: number };
  file: (path: string) => {
    exists: () => Promise<boolean>;
    text: () => Promise<string>;
  };
};

const execAsync = promisify(exec);
const DEV_PORT = 5180;
const root = import.meta.dir;
const distDir = join(root, 'dist');

let isBuilding = false;
let buildTimeout: ReturnType<typeof setTimeout> | null = null;
const clients: Set<{ write: (data: string) => void }> = new Set();

async function build(): Promise<void> {
  if (isBuilding) return;
  isBuilding = true;
  console.log('Building...');
  try {
    await execAsync('bun run build.ts', { cwd: root });
    console.log('Build complete');
    const msg = 'data: reload\n\n';
    clients.forEach((c) => c.write(msg));
  } catch (e) {
    const err = e as { stderr?: string; message?: string };
    console.error('Build failed:', err.stderr ?? err.message);
  } finally {
    isBuilding = false;
  }
}

watch(join(root, 'src'), { recursive: true }, (_event, filename) => {
  if (!filename) return;
  console.log(`Changed: ${filename}`);
  if (buildTimeout) clearTimeout(buildTimeout);
  buildTimeout = setTimeout(build, 200);
});

watch(join(root, 'index.html'), () => {
  if (buildTimeout) clearTimeout(buildTimeout);
  buildTimeout = setTimeout(build, 200);
});

await build();

const server = Bun.serve({
  port: DEV_PORT,
  async fetch(req: Request): Promise<Response> {
    const url = new URL(req.url);

    if (url.pathname === '/__dev_reload__') {
      return new Response(
        new ReadableStream({
          start(controller) {
            const client = {
              write: (data: string) =>
                controller.enqueue(new TextEncoder().encode(data)),
            };
            clients.add(client);
            req.signal.addEventListener('abort', () => clients.delete(client));
          },
        }),
        {
          headers: {
            'Content-Type': 'text/event-stream',
            'Cache-Control': 'no-cache',
          },
        },
      );
    }

    if (url.pathname === '/' || url.pathname === '') {
      let html = await Bun.file(join(distDir, 'index.html')).text();
      html = html.replace(
        '</body>',
        `<script>
          const es = new EventSource("/__dev_reload__");
          es.onmessage = () => location.reload();
        </script></body>`,
      );
      return new Response(html, { headers: { 'Content-Type': 'text/html' } });
    }

    const file = Bun.file(join(distDir, url.pathname));
    if (await file.exists()) return new Response(file as unknown as BodyInit);

    return new Response('Not found', { status: 404 });
  },
});

console.log(`tsot dev: http://localhost:${server.port}`);
