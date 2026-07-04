import { showErr } from "./overlay";

interface Bucket {
  count: number;
  bytes: number;
}

const buckets = new Map<string, Bucket>();
let firstCallsLogged = 0;
const FIRST_N_INDIVIDUAL = 30;
let allocCallsSinceLastSummary = 0;

function bump(key: string, bytes: number, detail: string): void {
  const b = buckets.get(key) ?? { count: 0, bytes: 0 };
  b.count += 1;
  b.bytes += bytes;
  buckets.set(key, b);
  allocCallsSinceLastSummary += 1;
  if (firstCallsLogged < FIRST_N_INDIVIDUAL) {
    firstCallsLogged += 1;
    const t = Math.round(performance.now());
    showErr(`[gpu-alloc.${key}@${t}ms] ${fmtMB(bytes)} ${detail}`);
  }
  if (allocCallsSinceLastSummary >= 20) emitSummary("burst");
}

function fmtMB(bytes: number): string {
  return `${(bytes / 1_048_576).toFixed(3)}MB`;
}

const WGPU_BPP: Record<string, number> = {
  "r8unorm": 1, "r8snorm": 1, "r8uint": 1, "r8sint": 1, "stencil8": 1,
  "r16uint": 2, "r16sint": 2, "r16float": 2,
  "rg8unorm": 2, "rg8snorm": 2, "rg8uint": 2, "rg8sint": 2,
  "depth16unorm": 2,
  "r32uint": 4, "r32sint": 4, "r32float": 4,
  "rg16uint": 4, "rg16sint": 4, "rg16float": 4,
  "rgba8unorm": 4, "rgba8unorm-srgb": 4, "rgba8snorm": 4,
  "rgba8uint": 4, "rgba8sint": 4,
  "bgra8unorm": 4, "bgra8unorm-srgb": 4,
  "rgb10a2unorm": 4, "rgb10a2uint": 4, "rg11b10ufloat": 4,
  "depth24plus": 4, "depth24plus-stencil8": 4, "depth32float": 4,
  "rg32uint": 8, "rg32sint": 8, "rg32float": 8,
  "rgba16uint": 8, "rgba16sint": 8, "rgba16float": 8,
  "depth32float-stencil8": 8,
  "rgba32uint": 16, "rgba32sint": 16, "rgba32float": 16,
};

function mipPyramidFactor(mips: number): number {
  let f = 0;
  for (let i = 0; i < mips; i++) f += 1 / (1 << (2 * i));
  return f;
}

interface Extent3D { width: number; height?: number; depthOrArrayLayers?: number }

function extentTo3(size: GPUTextureDescriptor["size"]): [number, number, number] {
  if (Array.isArray(size)) return [size[0], size[1] ?? 1, size[2] ?? 1];
  const e = size as Extent3D;
  return [e.width, e.height ?? 1, e.depthOrArrayLayers ?? 1];
}

function textureBytes(desc: GPUTextureDescriptor): { bytes: number; note: string } {
  const [w, h, d] = extentTo3(desc.size);
  const samples = desc.sampleCount ?? 1;
  const mips = desc.mipLevelCount ?? 1;
  const bpp = WGPU_BPP[desc.format as string];
  const note = `${w}x${h}x${d} fmt=${desc.format} samples=${samples} mips=${mips}`;
  if (bpp === undefined) return { bytes: 0, note };
  return { bytes: Math.round(w * h * d * bpp * samples * mipPyramidFactor(mips)), note };
}

function wgpuBufferTag(usage: number): string {
  if (usage & 0x0010) return "wgpu.buf.index";
  if (usage & 0x0020) return "wgpu.buf.vertex";
  if (usage & 0x0040) return "wgpu.buf.uniform";
  if (usage & 0x0080) return "wgpu.buf.storage";
  if (usage & 0x0100) return "wgpu.buf.indirect";
  return "wgpu.buf.other";
}

function wgpuTextureTag(usage: number): string {
  if (usage & 0x10) return "wgpu.tex.rt";
  if (usage & 0x04) return "wgpu.tex.sampled";
  if (usage & 0x08) return "wgpu.tex.storage";
  return "wgpu.tex.other";
}

function wrapDevice(device: GPUDevice): void {
  const origBuf = device.createBuffer;
  const origTex = device.createTexture;
  const origShader = device.createShaderModule;

  (device as { createBuffer: unknown }).createBuffer = function (this: GPUDevice, ...args: unknown[]) {
    const desc = args[0] as GPUBufferDescriptor;
    bump(wgpuBufferTag(desc.usage), desc.size, `usage=0x${desc.usage.toString(16)} label=${desc.label ?? "-"}`);
    return (origBuf as (...a: unknown[]) => GPUBuffer).apply(device, args);
  };

  (device as { createTexture: unknown }).createTexture = function (this: GPUDevice, ...args: unknown[]) {
    const desc = args[0] as GPUTextureDescriptor;
    const { bytes, note } = textureBytes(desc);
    if (bytes === 0) showErr(`[gpu-alloc.wgpu.tex.unknown] ${note} label=${desc.label ?? "-"}`);
    bump(wgpuTextureTag(desc.usage), bytes, `${note} label=${desc.label ?? "-"}`);
    return (origTex as (...a: unknown[]) => GPUTexture).apply(device, args);
  };

  (device as { createShaderModule: unknown }).createShaderModule = function (this: GPUDevice, ...args: unknown[]) {
    const desc = args[0] as GPUShaderModuleDescriptor;
    const size = desc.code.length;
    bump("wgpu.shader", size, `label=${desc.label ?? "-"} code_len=${size}`);
    return (origShader as (...a: unknown[]) => GPUShaderModule).apply(device, args);
  };
}

function wrapAdapter(adapter: GPUAdapter): void {
  const orig = adapter.requestDevice;
  (adapter as { requestDevice: unknown }).requestDevice = async function (this: GPUAdapter, ...args: unknown[]) {
    const dev = await (orig as (...a: unknown[]) => Promise<GPUDevice>).apply(adapter, args);
    wrapDevice(dev);
    showErr("[gpu-alloc] wgpu device wrapped");
    return dev;
  };
}

function installWebGPUProbe(): void {
  if (!("gpu" in navigator)) return;
  const gpu = navigator.gpu;
  const orig = gpu.requestAdapter;
  (gpu as { requestAdapter: unknown }).requestAdapter = async function (this: GPU, ...args: unknown[]) {
    const a = await (orig as (...a: unknown[]) => Promise<GPUAdapter | null>).apply(gpu, args);
    if (a) {
      wrapAdapter(a);
      showErr("[gpu-alloc] wgpu adapter wrapped");
    }
    return a;
  };
  showErr("[gpu-alloc] webgpu probe armed");
}

const GL_BPT: Record<number, number> = {
  0x1908: 4, 0x8058: 4, 0x8F97: 4, 0x8C43: 4,
  0x881A: 8, 0x8814: 16,
  0x1907: 3, 0x8051: 3, 0x881B: 6, 0x8815: 12,
  0x8229: 1, 0x8231: 1, 0x8232: 1,
  0x822A: 2, 0x822D: 2, 0x822B: 2, 0x822F: 4, 0x8230: 8,
  0x822E: 4,
  0x81A5: 2, 0x81A6: 4, 0x8CAC: 4, 0x88F0: 4, 0x8CAD: 8,
  0x8D48: 1,
  0x8C3A: 4, 0x8C3D: 4,
  0x8D62: 2,
};

function glTexBytes(fmt: number, w: number, h: number): { bytes: number; known: boolean } {
  const bpt = GL_BPT[fmt];
  if (bpt === undefined) return { bytes: 0, known: false };
  return { bytes: w * h * bpt, known: true };
}

function wrapWebGL2(gl: WebGL2RenderingContext): void {
  const origBuf = gl.bufferData;
  gl.bufferData = function (this: WebGL2RenderingContext, ...args: unknown[]): void {
    const target = args[0] as number;
    let size = 0;
    const src = args[1];
    if (typeof src === "number") {
      size = src;
    } else if (src && typeof src === "object") {
      const bv = src as ArrayBufferView;
      size = typeof bv.byteLength === "number" ? bv.byteLength : 0;
    }
    if (args.length >= 5) {
      const length = args[4] as number;
      const bv = args[1] as ArrayBufferView & { BYTES_PER_ELEMENT?: number };
      const bpe = bv && typeof bv.BYTES_PER_ELEMENT === "number" ? bv.BYTES_PER_ELEMENT : 1;
      if (typeof length === "number" && length > 0) size = length * bpe;
    }
    const tag = target === 0x8893 ? "gl.buf.index"
      : target === 0x8892 ? "gl.buf.vertex"
      : target === 0x8A11 ? "gl.buf.uniform"
      : `gl.buf.t${target.toString(16)}`;
    bump(tag, size, `argc=${args.length}`);
    return (origBuf as (...a: unknown[]) => void).apply(gl, args);
  } as unknown as WebGL2RenderingContext["bufferData"];

  const origStore = gl.texStorage2D;
  gl.texStorage2D = function (this: WebGL2RenderingContext, ...args: unknown[]): void {
    const levels = args[1] as number;
    const internalformat = args[2] as number;
    const width = args[3] as number;
    const height = args[4] as number;
    const { bytes, known } = glTexBytes(internalformat, width, height);
    if (!known) showErr(`[gpu-alloc.gl.tex.unknown] fmt=0x${internalformat.toString(16)} w=${width} h=${height}`);
    bump("gl.tex.storage", Math.round(bytes * mipPyramidFactor(levels)), `fmt=0x${internalformat.toString(16)} ${width}x${height} levels=${levels}`);
    return (origStore as (...a: unknown[]) => void).apply(gl, args);
  } as unknown as WebGL2RenderingContext["texStorage2D"];

  const origImg = gl.texImage2D;
  gl.texImage2D = function (this: WebGL2RenderingContext, ...args: unknown[]): void {
    let bytes = 0;
    let detail = `argc=${args.length}`;
    if (args.length >= 9) {
      const internalformat = args[2] as number;
      const width = args[3] as number;
      const height = args[4] as number;
      const r = glTexBytes(internalformat, width, height);
      bytes = r.bytes;
      detail = `9arg fmt=0x${internalformat.toString(16)} ${width}x${height}`;
    } else if (args.length === 6) {
      const source = args[5] as { width?: number; height?: number };
      const internalformat = args[2] as number;
      const w = source?.width ?? 0;
      const h = source?.height ?? 0;
      const r = glTexBytes(internalformat, w, h);
      bytes = r.bytes;
      detail = `6arg fmt=0x${internalformat.toString(16)} ${w}x${h}`;
    }
    if (bytes > 0) bump("gl.tex.image", bytes, detail);
    return (origImg as (...a: unknown[]) => void).apply(gl, args);
  } as unknown as WebGL2RenderingContext["texImage2D"];

  const origRB = gl.renderbufferStorage;
  gl.renderbufferStorage = function (this: WebGL2RenderingContext, ...args: unknown[]): void {
    const internalformat = args[1] as number;
    const width = args[2] as number;
    const height = args[3] as number;
    const { bytes } = glTexBytes(internalformat, width, height);
    bump("gl.rb", bytes, `fmt=0x${internalformat.toString(16)} ${width}x${height}`);
    return (origRB as (...a: unknown[]) => void).apply(gl, args);
  } as unknown as WebGL2RenderingContext["renderbufferStorage"];

  const origRBMS = gl.renderbufferStorageMultisample;
  gl.renderbufferStorageMultisample = function (this: WebGL2RenderingContext, ...args: unknown[]): void {
    const samples = args[1] as number;
    const internalformat = args[2] as number;
    const width = args[3] as number;
    const height = args[4] as number;
    const { bytes } = glTexBytes(internalformat, width, height);
    bump("gl.rb.ms", bytes * samples, `fmt=0x${internalformat.toString(16)} ${width}x${height} samples=${samples}`);
    return (origRBMS as (...a: unknown[]) => void).apply(gl, args);
  } as unknown as WebGL2RenderingContext["renderbufferStorageMultisample"];

  const origShaderSource = gl.shaderSource;
  gl.shaderSource = function (this: WebGL2RenderingContext, ...args: unknown[]): void {
    const src = args[1] as string;
    const size = typeof src === "string" ? src.length : 0;
    bump("gl.shader", size, `code_len=${size}`);
    return (origShaderSource as (...a: unknown[]) => void).apply(gl, args);
  } as unknown as WebGL2RenderingContext["shaderSource"];
}

function installWebGL2Probe(): void {
  const orig = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement, ...args: unknown[]): unknown {
    const ctx = (orig as (...a: unknown[]) => unknown).apply(this, args);
    if (args[0] === "webgl2" && ctx) {
      wrapWebGL2(ctx as WebGL2RenderingContext);
      showErr("[gpu-alloc] webgl2 context wrapped");
    }
    return ctx;
  } as typeof HTMLCanvasElement.prototype.getContext;
  showErr("[gpu-alloc] webgl2 probe armed");
}

function emitSummary(reason: string): void {
  allocCallsSinceLastSummary = 0;
  const t = Math.round(performance.now() / 1000);
  let total = 0;
  const entries: Array<[string, Bucket]> = [];
  for (const [k, v] of buckets) {
    total += v.bytes;
    entries.push([k, v]);
  }
  if (entries.length === 0) {
    showErr(`[gpu-alloc@${t}s ${reason}] total=0.000MB (no allocations yet)`);
    return;
  }
  entries.sort((a, b) => b[1].bytes - a[1].bytes);
  const parts = entries
    .map(([k, v]) => `${k}=${fmtMB(v.bytes)}/${v.count}`)
    .join(" ");
  const pcts = entries
    .map(([k, v]) => `${k}=${((v.bytes / total) * 100).toFixed(1)}%`)
    .join(" ");
  showErr(`[gpu-alloc@${t}s ${reason}] total=${fmtMB(total)} · ${parts}`);
  showErr(`[gpu-alloc@${t}s ${reason} %] ${pcts}`);
}

export function installGpuAllocProbe(): void {
  installWebGPUProbe();
  installWebGL2Probe();
  window.setInterval(() => emitSummary("tick"), 5000);
}

export function forceEmitSummary(reason: string): void {
  emitSummary(reason);
}
