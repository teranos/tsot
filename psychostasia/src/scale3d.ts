import * as THREE from 'three'
import { RoomEnvironment } from 'three/examples/jsm/environments/RoomEnvironment.js'
import { OBJLoader } from 'three/examples/jsm/loaders/OBJLoader.js'
import type { FieldParams } from './field'
import { DEFAULT_LOOK, type Look } from './look'
import { type Point, swing, yaw } from './rig'

// The stage: heaven's field behind, the brass scale (assets/scale/scale.obj,
// Y-up) in front. The scale's parts are split by name in the OBJ: the beam
// turns on scale_beam_axis; everything left of the column hangs from
// scale_double_ring_01, everything right from scale_double_ring_03; the
// column itself stays put. Every adjustable number comes from the look.

// Parts whose centre lies within this of the column are fixed.
const COLUMN_HALF_WIDTH = 1
const CENTRE_Y = 17
const DEG = Math.PI / 180

export interface Scale3d {
  setSoul(soul: number): void
  setField(p: FieldParams): void
  setLook(look: Look): void
}

type OnError = (where: string, e: unknown) => void

export function createScale3d(canvas: HTMLCanvasElement, onError: OnError): Scale3d {
  const renderer = new THREE.WebGLRenderer({ canvas, alpha: true, antialias: true })
  renderer.setPixelRatio(window.devicePixelRatio)
  renderer.outputColorSpace = THREE.SRGBColorSpace

  const scene = new THREE.Scene()
  // Metal shows what it reflects; without an environment brass reads black.
  scene.environment = new THREE.PMREMGenerator(renderer).fromScene(new RoomEnvironment(), 0.04).texture
  const key = new THREE.DirectionalLight(0xffffff, 1.5)
  key.position.set(10, 40, 30)
  scene.add(key)

  // Framed so a fully sunk pan stays on screen.
  const camera = new THREE.PerspectiveCamera(35, 1, 1, 500)
  camera.position.set(0, CENTRE_Y, 110)
  camera.lookAt(0, CENTRE_Y, 0)

  let look: Look = { ...DEFAULT_LOOK }
  const field = heavenField()
  scene.add(field.mesh)

  // Everything turns together about the column, away from the cursor.
  const root = new THREE.Group()
  scene.add(root)
  let cursorX = window.innerWidth / 2
  window.addEventListener('pointermove', ev => {
    cursorX = ev.clientX
  })

  const resize = () => {
    renderer.setSize(window.innerWidth, window.innerHeight, false)
    camera.aspect = window.innerWidth / window.innerHeight
    camera.updateProjectionMatrix()
    field.setAspect(camera.aspect)
  }
  resize()
  window.addEventListener('resize', resize)

  const fail = (url: string) => (e: unknown) => onError(`scale: ${url} failed to load`, e)
  const textures = new THREE.TextureLoader()
  const map = textures.load('/scale/diffuse.jpg', undefined, undefined, fail('/scale/diffuse.jpg'))
  map.colorSpace = THREE.SRGBColorSpace
  const normalMap = textures.load('/scale/normal.jpg', undefined, undefined, fail('/scale/normal.jpg'))
  const brass = new THREE.MeshStandardMaterial({ map, normalMap, metalness: 0.85, roughness: 0.35 })

  let soul = 0
  let setAngle: ((angle: number) => void) | null = null

  new OBJLoader().load(
    '/scale/scale.obj',
    obj => {
      try {
        setAngle = rig(obj)
      } catch (e) {
        onError('scale: rigging scale.obj failed', e)
      }
    },
    undefined,
    fail('/scale/scale.obj'),
  )

  function rig(obj: THREE.Group): (angle: number) => void {
    const meshes = obj.children.filter((c): c is THREE.Mesh => (c as THREE.Mesh).isMesh)
    const part = (name: string) => {
      const m = meshes.find(x => x.name === name)
      if (!m) throw new Error(`part ${name} is not in scale.obj (has: ${meshes.map(x => x.name).join(', ')})`)
      return m
    }
    const bounds = (m: THREE.Object3D) => new THREE.Box3().setFromObject(m)
    const axis = bounds(part('scale_beam_axis')).getCenter(new THREE.Vector3())
    const pivot: Point = { x: axis.x, y: axis.y }
    const hook = (name: string): Point => {
      const b = bounds(part(name))
      return { x: (b.min.x + b.max.x) / 2, y: b.max.y }
    }
    const leftHook = hook('scale_double_ring_01')
    const rightHook = hook('scale_double_ring_03')
    const leftPan = bounds(part('scale_pan_01'))
    const rightPan = bounds(part('scale_pan_02'))

    const beam = new THREE.Group()
    beam.position.copy(axis)
    const left = new THREE.Group()
    const right = new THREE.Group()
    const column = new THREE.Group()
    for (const m of meshes) {
      m.material = brass
      if (m.name === 'scale_beam') {
        m.position.sub(axis)
        beam.add(m)
        continue
      }
      const x = bounds(m).getCenter(new THREE.Vector3()).x
      if (Math.abs(x) < COLUMN_HALF_WIDTH) column.add(m)
      else if (x < 0) left.add(m)
      else right.add(m)
    }

    const heart = new THREE.Mesh(heartGeometry(), new THREE.MeshStandardMaterial({ color: 0x8b1a1a, roughness: 0.5 }))
    heart.scale.setScalar(1.3)
    heart.position.set((leftPan.min.x + leftPan.max.x) / 2, leftPan.max.y + 2.4, (leftPan.min.z + leftPan.max.z) / 2)
    left.add(heart)

    const feather = new THREE.Mesh(
      new THREE.SphereGeometry(1, 24, 12),
      new THREE.MeshStandardMaterial({ color: 0xf5f0e1, roughness: 0.8 }),
    )
    feather.scale.set(4, 0.35, 1.3)
    feather.rotation.z = 0.15
    feather.position.set((rightPan.min.x + rightPan.max.x) / 2, rightPan.max.y + 0.5, (rightPan.min.z + rightPan.max.z) / 2)
    right.add(feather)

    root.add(column, beam, left, right)

    return angle => {
      // three turns counter-clockwise for positive z; swing raises the
      // left side for positive angles, so the beam turns the other way.
      beam.rotation.z = -angle
      const l = swing(leftHook, pivot, angle)
      const r = swing(rightHook, pivot, angle)
      left.position.set(l.x - leftHook.x, l.y - leftHook.y, 0)
      right.position.set(r.x - rightHook.x, r.y - rightHook.y, 0)
    }
  }

  let last = performance.now()
  renderer.setAnimationLoop(now => {
    const dt = Math.min(0.1, (now - last) / 1000)
    last = now
    // Ease toward the cursor so the turn follows without jumping.
    const target = yaw(cursorX, window.innerWidth, look.yawMax * DEG)
    root.rotation.y += (target - root.rotation.y) * Math.min(1, dt * 6)
    setAngle?.(soul * look.tilt * DEG)
    field.tick(dt, look)
    renderer.render(scene, camera)
  })

  return {
    setSoul(v: number) {
      soul = v
    },
    setField(p: FieldParams) {
      field.set(p)
    },
    setLook(l: Look) {
      look = l
    },
  }
}

// Heaven: a fullscreen breakthrough field. An endless log-polar tunnel,
// kaleidoscope mirrors, fractal folds and iridescent colour, all shaped by
// the look and driven by the sound. Drawn at the far plane so everything
// else stands in front of it.
function heavenField() {
  const uniforms = {
    uPhase: { value: 0 },
    uAlpha: { value: 0 },
    uSegments: { value: 8 },
    uPulse: { value: 0 },
    uAspect: { value: 1 },
    uFolds: { value: 3 },
    uFoldX: { value: 0.9 },
    uFoldY: { value: 0.6 },
    uWobble: { value: 0.15 },
    uZoom: { value: 1 },
    uRingFreq: { value: 6 },
    uRingDepth: { value: 0.4 },
    uHue: { value: 0 },
    uColorSpeed: { value: 0.2 },
    uSpread: { value: 0.3 },
    uBright: { value: 1 },
    uCore: { value: 0.15 },
  }
  const material = new THREE.ShaderMaterial({
    uniforms,
    transparent: true,
    depthWrite: false,
    vertexShader: /* glsl */ `
      varying vec2 vUv;
      void main() {
        vUv = uv;
        gl_Position = vec4(position.xy, 1.0, 1.0);
      }
    `,
    fragmentShader: /* glsl */ `
      uniform float uPhase, uAlpha, uSegments, uPulse, uAspect;
      uniform float uFolds, uFoldX, uFoldY, uWobble, uZoom, uRingFreq, uRingDepth;
      uniform float uHue, uColorSpeed, uSpread, uBright, uCore;
      varying vec2 vUv;
      const float TAU = 6.28318;
      vec3 pal(float t) { return 0.5 + 0.5 * cos(TAU * (t + vec3(0.0, 0.33, 0.67))); }
      vec2 kale(vec2 p, float n) {
        float s = TAU / n;
        float a = mod(atan(p.y, p.x), s);
        a = abs(a - s * 0.5);
        return vec2(cos(a), sin(a)) * length(p);
      }
      void main() {
        vec2 p = (vUv - 0.5) * vec2(uAspect, 1.0) * 2.0 / uZoom;
        float t = uPhase;
        float r = length(p);
        // Endless zoom: log-polar depth falls toward the centre forever.
        float depth = log(r + 1e-4) - t;
        vec3 col = vec3(0.0);
        vec2 q = p * (1.0 + 0.25 * uPulse);
        for (int i = 0; i < 6; i++) {
          if (float(i) >= uFolds) break;
          q = kale(q, uSegments);
          q = abs(q) / dot(q, q) - vec2(uFoldX + uWobble * sin(t * 0.7 + float(i)), uFoldY + uWobble * 1.3 * cos(t * 0.5));
          float k = length(q);
          col += pal(uHue + k * uSpread + t * uColorSpeed + float(i) * 0.15 + depth * 0.1) * exp(-k * 0.8);
        }
        col *= (1.0 - uRingDepth) + uRingDepth * sin(depth * uRingFreq + uPulse * 4.0);
        col += pal(uHue + depth * 0.5 - t) * uCore / (r + 0.2);
        gl_FragColor = vec4(col * uBright, uAlpha);
      }
    `,
  })
  const mesh = new THREE.Mesh(new THREE.PlaneGeometry(2, 2), material)
  mesh.frustumCulled = false
  let speed = 0.15
  return {
    mesh,
    setAspect(a: number) {
      uniforms.uAspect.value = a
    },
    set(p: FieldParams) {
      uniforms.uSegments.value = p.segments
      uniforms.uPulse.value = p.pulse
      uniforms.uAlpha.value = p.alpha
      speed = p.speed
    },
    tick(dt: number, look: Look) {
      // Phase accumulates so a change of speed never jumps the tunnel.
      uniforms.uPhase.value += dt * speed
      uniforms.uFolds.value = look.folds
      uniforms.uFoldX.value = look.foldX
      uniforms.uFoldY.value = look.foldY
      uniforms.uWobble.value = look.wobble
      uniforms.uZoom.value = look.zoom
      uniforms.uRingFreq.value = look.ringFreq
      uniforms.uRingDepth.value = look.ringDepth
      uniforms.uHue.value = look.hue
      uniforms.uColorSpeed.value = look.colorSpeed
      uniforms.uSpread.value = look.spread
      uniforms.uBright.value = look.brightness
      uniforms.uCore.value = look.core
    },
  }
}

function heartGeometry(): THREE.BufferGeometry {
  const s = new THREE.Shape()
  s.moveTo(0, 0.5)
  s.bezierCurveTo(0, 0.8, -0.5, 1.2, -1, 1.2)
  s.bezierCurveTo(-1.8, 1.2, -1.8, 0.3, -1.8, 0.3)
  s.bezierCurveTo(-1.8, -0.5, -0.8, -1.2, 0, -1.8)
  s.bezierCurveTo(0.8, -1.2, 1.8, -0.5, 1.8, 0.3)
  s.bezierCurveTo(1.8, 0.3, 1.8, 1.2, 1, 1.2)
  s.bezierCurveTo(0.5, 1.2, 0, 0.8, 0, 0.5)
  const g = new THREE.ExtrudeGeometry(s, { depth: 0.8, bevelEnabled: true, bevelSize: 0.15, bevelThickness: 0.15 })
  g.center()
  return g
}
