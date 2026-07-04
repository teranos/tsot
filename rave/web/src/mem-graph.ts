import { getBucketTotals } from "./gpu-alloc";
import { showErr } from "./overlay";

const WIDTH = 320;
const HEIGHT = 200;
const MAX_SAMPLES = 120;
const SAMPLE_MS = 500;

interface Series {
  color: string;
  data: number[];
}

const seriesMap = new Map<string, Series>();

const COLORS = [
  "#ff5", "#5ff", "#f5f", "#fa5", "#5af", "#a5f",
  "#faa", "#afa", "#aaf", "#fca", "#cfa", "#acf",
  "#f77", "#7f7", "#77f", "#fc7", "#7fc", "#c7f",
];
let colorIdx = 0;

function nextColor(): string {
  const c = COLORS[colorIdx % COLORS.length];
  colorIdx += 1;
  return c;
}

function pushSample(key: string, mb: number): void {
  let s = seriesMap.get(key);
  if (!s) {
    s = { color: nextColor(), data: [] };
    seriesMap.set(key, s);
  }
  s.data.push(mb);
  if (s.data.length > MAX_SAMPLES) s.data.shift();
}

function maxOfAll(): number {
  let max = 0.1;
  for (const s of seriesMap.values()) {
    for (const v of s.data) if (v > max) max = v;
  }
  return max;
}

function draw(canvas: HTMLCanvasElement): void {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.fillStyle = "rgba(0, 0, 0, 0.82)";
  ctx.fillRect(0, 0, WIDTH, HEIGHT);
  const max = maxOfAll();
  const xStep = WIDTH / MAX_SAMPLES;

  ctx.fillStyle = "#888";
  ctx.font = "10px ui-monospace, monospace";
  ctx.fillText(`${max.toFixed(1)} MB max`, WIDTH - 90, 12);

  const sortedKeys = Array.from(seriesMap.keys()).sort((a, b) => {
    const av = (seriesMap.get(a)?.data.slice(-1)[0] ?? 0);
    const bv = (seriesMap.get(b)?.data.slice(-1)[0] ?? 0);
    return bv - av;
  });

  for (const key of sortedKeys) {
    const s = seriesMap.get(key);
    if (!s || s.data.length === 0) continue;
    ctx.strokeStyle = s.color;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    s.data.forEach((v, i) => {
      const x = i * xStep;
      const y = HEIGHT - (v / max) * (HEIGHT - 8) - 4;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    });
    ctx.stroke();
  }

  let ly = 24;
  for (const key of sortedKeys) {
    const s = seriesMap.get(key);
    if (!s || s.data.length === 0) continue;
    const last = s.data[s.data.length - 1];
    ctx.fillStyle = s.color;
    ctx.fillText(`${key}: ${last.toFixed(2)}MB`, 6, ly);
    ly += 12;
    if (ly > HEIGHT - 6) break;
  }
}

export function installMemGraph(getWasmBytes: () => number): void {
  const canvas = document.createElement("canvas");
  canvas.width = WIDTH;
  canvas.height = HEIGHT;
  canvas.id = "mem-graph";
  canvas.style.position = "fixed";
  canvas.style.bottom = "88px";
  canvas.style.right = "6px";
  canvas.style.zIndex = "10000";
  canvas.style.border = "1px solid #555";
  canvas.style.pointerEvents = "none";
  document.body.appendChild(canvas);

  window.setInterval(() => {
    const t = getBucketTotals();
    for (const [k, v] of Object.entries(t)) pushSample(k, v / 1_048_576);
    pushSample("wasm.linear", getWasmBytes() / 1_048_576);
    draw(canvas);
  }, SAMPLE_MS);

  showErr("[mem-graph] installed 320x200 bottom-right");
}
