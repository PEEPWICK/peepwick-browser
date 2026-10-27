/// <reference lib="dom" />
// Fingerprinting protection, after Brave's "farbling": values that let a site tell
// one browser from another (canvas and WebGL pixels, audio samples, the GPU model,
// the processor count) get small changes no person would notice. The changes depend
// on a seed that differs for every site and every browser session, so sites can't
// match the same person across sites or between sessions, while one site sees
// steady values for as long as the browser stays open.
//
// farble() runs in the page's own world (contextBridge.executeInMainWorld) before
// any of the page's scripts. It's sent there as source text, so it must not use
// anything from outside its own body.

export function farble(seed: number): void {
  const nativeToString = Function.prototype.toString
  /** Replaced functions and getters, with the source text the originals show. */
  const natives = new WeakMap<object, string>()

  const mix = (value: number): number => {
    let n = Math.imul(value ^ (value >>> 16), 0x45d9f3b)
    n = Math.imul(n ^ (n >>> 16), 0x45d9f3b)
    return (n ^ (n >>> 16)) >>> 0
  }

  type AnyFn = (this: any, ...args: any[]) => any

  /** Swaps a method for one that looks like the original: same name, length and source text. */
  const patch = (target: object | undefined, name: string, make: (original: AnyFn) => AnyFn): void => {
    if (!target) return
    const descriptor = Object.getOwnPropertyDescriptor(target, name)
    const original = descriptor?.value as AnyFn | undefined
    if (!descriptor || typeof original !== 'function') return
    const wrapped = make(original)
    // A method definition can't be called with `new` and has no prototype, like a native method.
    const replacement = {
      [name](this: unknown, ...args: unknown[]) {
        return wrapped.apply(this, args)
      },
    }[name]
    Object.defineProperty(replacement, 'length', { value: original.length })
    natives.set(replacement, nativeToString.call(original))
    Object.defineProperty(target, name, { ...descriptor, value: replacement })
  }

  const patchGetter = (target: object | undefined, name: string, make: (original: () => unknown) => () => unknown): void => {
    if (!target) return
    const descriptor = Object.getOwnPropertyDescriptor(target, name)
    const original = descriptor?.get
    if (!descriptor || !original) return
    const wrapped = make(original)
    const getter = Object.getOwnPropertyDescriptor(
      {
        get [name]() {
          return wrapped.call(this)
        },
      },
      name,
    )!.get!
    natives.set(getter, nativeToString.call(original))
    Object.defineProperty(target, name, { ...descriptor, get: getter })
  }

  patch(Function.prototype, 'toString', (original) =>
    function (this: unknown) {
      return natives.get(this as object) ?? original.call(this)
    },
  )

  // ── Canvas ─────────────────────────────────────────────────────────────────

  /** Flips the lowest bit of one colour in about one opaque pixel out of fifty. */
  const noise = (data: Uint8Array | Uint8ClampedArray): void => {
    for (let i = 0; i + 3 < data.length; i += 4) {
      if (data[i + 3] !== 255) continue
      const h = mix(seed ^ Math.imul(i, 0x27d4eb2d) ^ (data[i] << 16) ^ (data[i + 1] << 8) ^ data[i + 2])
      if (h % 50 === 0) data[i + ((h >>> 8) % 3)] ^= 1
    }
  }

  const getImageData2D = CanvasRenderingContext2D.prototype.getImageData
  const drawImage2D = CanvasRenderingContext2D.prototype.drawImage as (this: CanvasRenderingContext2D, image: CanvasImageSource, x: number, y: number) => void
  const putImageData2D = CanvasRenderingContext2D.prototype.putImageData as (this: CanvasRenderingContext2D, image: ImageData, x: number, y: number) => void

  /** A copy of a canvas with the noise added, or null to read the original (and get its errors). */
  const noisyCopy = (canvas: HTMLCanvasElement): HTMLCanvasElement | null => {
    const { width, height } = canvas
    if (!width || !height || width * height > 16_000_000) return null
    try {
      const copy = document.createElement('canvas')
      copy.width = width
      copy.height = height
      const context = copy.getContext('2d')
      if (!context) return null
      drawImage2D.call(context, canvas, 0, 0)
      const image = getImageData2D.call(context, 0, 0, width, height)
      noise(image.data)
      putImageData2D.call(context, image, 0, 0)
      return copy
    } catch {
      return null
    }
  }

  patch(HTMLCanvasElement.prototype, 'toDataURL', (original) =>
    function (this: HTMLCanvasElement, ...args: unknown[]) {
      return original.apply(noisyCopy(this) ?? this, args)
    },
  )
  patch(HTMLCanvasElement.prototype, 'toBlob', (original) =>
    function (this: HTMLCanvasElement, ...args: unknown[]) {
      return original.apply(noisyCopy(this) ?? this, args)
    },
  )
  for (const proto of [CanvasRenderingContext2D.prototype, globalThis.OffscreenCanvasRenderingContext2D?.prototype]) {
    patch(proto, 'getImageData', (original) =>
      function (this: CanvasRenderingContext2D, ...args: unknown[]) {
        const image = original.apply(this, args) as ImageData
        noise(image.data)
        return image
      },
    )
  }

  // ── WebGL ──────────────────────────────────────────────────────────────────

  const UNMASKED_VENDOR = 0x9245
  const UNMASKED_RENDERER = 0x9246
  for (const proto of [globalThis.WebGLRenderingContext?.prototype, globalThis.WebGL2RenderingContext?.prototype]) {
    patch(proto, 'readPixels', (original) =>
      function (this: WebGLRenderingContext, ...args: unknown[]) {
        const result = original.apply(this, args)
        const pixels = args[6]
        if (pixels instanceof Uint8Array) noise(pixels)
        return result
      },
    )
    // Keep the GPU maker (sites pick graphics quality by it), drop the exact model and driver.
    patch(proto, 'getParameter', (original) =>
      function (this: WebGLRenderingContext, parameter: unknown) {
        const value = original.call(this, parameter)
        if (parameter === UNMASKED_RENDERER && typeof value === 'string') {
          const maker = value.match(/^ANGLE \(([^,]+),/)?.[1]
          return maker ? `ANGLE (${maker}, Generic Renderer)` : 'ANGLE (Generic Renderer)'
        }
        if (parameter === UNMASKED_VENDOR && typeof value === 'string') return 'Google Inc.'
        return value
      },
    )
  }

  // ── Audio ──────────────────────────────────────────────────────────────────

  // Off by at most a hundredth of a percent: far below hearing, enough to change a hash.
  const gain = 1 + (((seed % 997) + 1) / 997) * 1e-4 * (seed & 1 ? 1 : -1)
  const scaled = new WeakSet<Float32Array>()
  patch(globalThis.AudioBuffer?.prototype, 'getChannelData', (original) =>
    function (this: AudioBuffer, ...args: unknown[]) {
      const data = original.apply(this, args) as Float32Array
      if (!scaled.has(data)) {
        scaled.add(data)
        for (let i = 0; i < data.length; i++) data[i] *= gain
      }
      return data
    },
  )
  patch(globalThis.AudioBuffer?.prototype, 'copyFromChannel', (original) =>
    function (this: AudioBuffer, ...args: unknown[]) {
      const result = original.apply(this, args)
      const destination = args[0]
      if (destination instanceof Float32Array) for (let i = 0; i < destination.length; i++) destination[i] *= gain
      return result
    },
  )
  patch(globalThis.AnalyserNode?.prototype, 'getFloatFrequencyData', (original) =>
    function (this: AnalyserNode, ...args: unknown[]) {
      const result = original.apply(this, args)
      const array = args[0]
      if (array instanceof Float32Array) for (let i = 0; i < array.length; i++) array[i] *= gain
      return result
    },
  )

  // ── Hardware ───────────────────────────────────────────────────────────────

  // Processor counts in a few common steps instead of the exact number.
  patchGetter(Navigator.prototype, 'hardwareConcurrency', (original) =>
    function (this: Navigator) {
      const cores = original.call(this) as number
      return cores >= 8 ? 8 : cores >= 4 ? 4 : cores
    },
  )
}
