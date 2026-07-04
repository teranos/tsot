import { getBucketTotals } from "./gpu-alloc";
import { showErr } from "./overlay";

const WIDTH = 640;
const HEIGHT = 220;
const LEGEND_WIDTH = 240;
const PLOT_WIDTH = WIDTH - LEGEND_WIDTH;
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
  ctx.clearRect(0, 0, WIDTH, HEIGHT);
  const max = maxOfAll();
  const xStep = PLOT_WIDTH / MAX_SAMPLES;

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
    ctx.shadowColor = "rgba(0,0,0,0.8)";
    ctx.shadowBlur = 3;
    ctx.beginPath();
    s.data.forEach((v, i) => {
      const x = i * xStep;
      const y = HEIGHT - (v / max) * (HEIGHT - 12) - 6;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    });
    ctx.stroke();
  }
  ctx.shadowBlur = 0;

  ctx.fillStyle = "#888";
  ctx.font = "10px ui-monospace, monospace";
  ctx.shadowColor = "rgba(0,0,0,0.9)";
  ctx.shadowBlur = 3;
  ctx.fillText(`${max.toFixed(1)} MB max`, PLOT_WIDTH - 90, 12);

  const legendX = PLOT_WIDTH + 10;
  let ly = 14;
  for (const key of sortedKeys) {
    const s = seriesMap.get(key);
    if (!s || s.data.length === 0) continue;
    const last = s.data[s.data.length - 1];
    ctx.fillStyle = s.color;
    ctx.fillText(`${key}: ${last.toFixed(2)}MB`, legendX, ly);
    ly += 13;
    if (ly > HEIGHT - 6) break;
  }
  ctx.shadowBlur = 0;
}

export function installMemGraph(getWasmBytes: () => number): void {
  const canvas = document.createElement("canvas");
  canvas.width = WIDTH;
  canvas.height = HEIGHT;
  canvas.id = "mem-graph";
  canvas.style.position = "fixed";
  canvas.style.top = "0";
  canvas.style.right = "0";
  canvas.style.width = `${WIDTH}px`;
  canvas.style.height = `${HEIGHT}px`;
  canvas.style.maxWidth = "calc(100vw - 4px)";
  canvas.style.zIndex = "10000";
  canvas.style.pointerEvents = "none";
  canvas.style.background = "transparent";
  document.body.appendChild(canvas);

  window.setInterval(() => {
    const t = getBucketTotals();
    for (const [k, v] of Object.entries(t)) pushSample(k, v / 1_048_576);
    pushSample("wasm.linear", getWasmBytes() / 1_048_576);
    draw(canvas);
  }, SAMPLE_MS);

  showErr(`[mem-graph] installed ${WIDTH}x${HEIGHT} top-right, transparent, legend right`);
}
