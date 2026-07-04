import { showErr } from "./overlay";

interface Bucket {
  count: number;
  bytes: number;
}

const buckets = new Map<string, Bucket>();
let allocCallsSinceLastSummary = 0;

function bump(key: string, bytes: number): void {
  const b = buckets.get(key) ?? { count: 0, bytes: 0 };
  b.count += 1;
  b.bytes += bytes;
  buckets.set(key, b);
  allocCallsSinceLastSummary += 1;
  if (allocCallsSinceLastSummary >= 20) {
    emitSummary("burst");
  }
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
  if (Array.isArray(size)) {
    return [size[0], size[1] ?? 1, size[2] ?? 1];
  }
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
  const bytes = Math.round(w * h * d * bpp * samples * mipPyramidFactor(mips));
  return { bytes, note };
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
  const origBuf = device.createBuffer.bind(device);
  const origTex = device.createTexture.bind(device);
  (device as { createBuffer: typeof device.createBuffer }).createBuffer = ((
    desc: GPUBufferDescriptor,
  ) => {
    bump(wgpuBufferTag(desc.usage), desc.size);
    return origBuf(desc);
  }) as typeof device.createBuffer;
  (device as { createTexture: typeof device.createTexture }).createTexture = ((
    desc: GPUTextureDescriptor,
  ) => {
    const { bytes, note } = textureBytes(desc);
    if (bytes === 0) showErr(`[gpu-alloc.wgpu.tex.unknown] ${note}`);
    bump(wgpuTextureTag(desc.usage), bytes);
    return origTex(desc);
  }) as typeof device.createTexture;
}

function wrapAdapter(adapter: GPUAdapter): void {
  const orig = adapter.requestDevice.bind(adapter);
  (adapter as { requestDevice: typeof adapter.requestDevice }).requestDevice = (async (
    desc?: GPUDeviceDescriptor,
  ) => {
    const dev = await orig(desc);
    wrapDevice(dev);
    showErr("[gpu-alloc] wgpu device wrapped");
    return dev;
  }) as typeof adapter.requestDevice;
}

function installWebGPUProbe(): void {
  if (!("gpu" in navigator)) return;
  const gpu = navigator.gpu;
  const orig = gpu.requestAdapter.bind(gpu);
  (gpu as { requestAdapter: typeof gpu.requestAdapter }).requestAdapter = (async (
    opts?: GPURequestAdapterOptions,
  ) => {
    const a = await orig(opts);
    if (a) {
      wrapAdapter(a);
      showErr("[gpu-alloc] wgpu adapter wrapped");
    }
    return a;
  }) as typeof gpu.requestAdapter;
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
  const origBuf = gl.bufferData.bind(gl);
  (gl as { bufferData: WebGL2RenderingContext["bufferData"] }).bufferData = ((
    target: number,
    sizeOrData: number | ArrayBufferView | ArrayBuffer | null,
    usage: number,
  ) => {
    let size = 0;
    if (typeof sizeOrData === "number") size = sizeOrData;
    else if (sizeOrData) {
      const bv = sizeOrData as ArrayBufferView;
      const ab = sizeOrData as ArrayBuffer;
      size = bv.byteLength ?? ab.byteLength ?? 0;
    }
    const tag =
      target === 0x8893 ? "gl.buf.index" :
      target === 0x8892 ? "gl.buf.vertex" :
      target === 0x8A11 ? "gl.buf.uniform" :
      "gl.buf.other";
    bump(tag, size);
    return origBuf(target, sizeOrData as ArrayBufferView, usage);
  }) as WebGL2RenderingContext["bufferData"];

  const origStore = gl.texStorage2D.bind(gl);
  (gl as { texStorage2D: WebGL2RenderingContext["texStorage2D"] }).texStorage2D = ((
    target: number, levels: number, internalformat: number, width: number, height: number,
  ) => {
    const { bytes, known } = glTexBytes(internalformat, width, height);
    if (!known) showErr(`[gpu-alloc.gl.tex.unknown] fmt=0x${internalformat.toString(16)} w=${width} h=${height}`);
    bump("gl.tex.storage", Math.round(bytes * mipPyramidFactor(levels)));
    return origStore(target, levels, internalformat, width, height);
  }) as WebGL2RenderingContext["texStorage2D"];

  const origImg2D = gl.texImage2D.bind(gl);
  const wrappedImg = function (...args: unknown[]): void {
    if (args.length >= 5) {
      const internalformat = args[2] as number;
      const width = args[3] as number;
      const height = args[4] as number;
      const { bytes } = glTexBytes(internalformat, width, height);
      if (bytes > 0) bump("gl.tex.image", bytes);
    }
    return (origImg2D as (...a: unknown[]) => void)(...args);
  };
  (gl as unknown as { texImage2D: unknown }).texImage2D = wrappedImg;

  const origRB = gl.renderbufferStorage.bind(gl);
  (gl as { renderbufferStorage: WebGL2RenderingContext["renderbufferStorage"] }).renderbufferStorage = ((
    target: number, internalformat: number, width: number, height: number,
  ) => {
    const { bytes } = glTexBytes(internalformat, width, height);
    bump("gl.rb", bytes);
    return origRB(target, internalformat, width, height);
  }) as WebGL2RenderingContext["renderbufferStorage"];

  const origRBMS = gl.renderbufferStorageMultisample.bind(gl);
  (gl as { renderbufferStorageMultisample: WebGL2RenderingContext["renderbufferStorageMultisample"] }).renderbufferStorageMultisample = ((
    target: number, samples: number, internalformat: number, width: number, height: number,
  ) => {
    const { bytes } = glTexBytes(internalformat, width, height);
    bump("gl.rb.ms", bytes * samples);
    return origRBMS(target, samples, internalformat, width, height);
  }) as WebGL2RenderingContext["renderbufferStorageMultisample"];
}

function installWebGL2Probe(): void {
  const orig = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function (
    this: HTMLCanvasElement,
    id: string,
    opts?: unknown,
  ): unknown {
    const ctx = (orig as (id: string, opts?: unknown) => unknown).call(this, id, opts);
    if (id === "webgl2" && ctx) {
      wrapWebGL2(ctx as WebGL2RenderingContext);
      showErr("[gpu-alloc] webgl2 context wrapped");
    }
    return ctx;
  } as typeof HTMLCanvasElement.prototype.getContext;
  showErr("[gpu-alloc] webgl2 probe armed");
}

function fmtMB(bytes: number): string {
  return `${(bytes / 1_048_576).toFixed(2)}MB`;
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
  entries.sort((a, b) => b[1].bytes - a[1].bytes);
  const parts = entries.map(([k, v]) => `${k}=${fmtMB(v.bytes)}/${v.count}`).join(" ");
  showErr(`[gpu-alloc@${t}s ${reason}] total=${fmtMB(total)} · ${parts}`);
}

export function installGpuAllocProbe(): void {
  installWebGPUProbe();
  installWebGL2Probe();
  window.setInterval(() => emitSummary("tick"), 5000);
}
