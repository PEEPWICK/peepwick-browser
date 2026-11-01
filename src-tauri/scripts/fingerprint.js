// Fingerprinting protection, after Brave's "farbling": values that let a site tell one
// browser from another (canvas and WebGL pixels, audio samples, the GPU model, the processor
// count) get small changes no person would notice. The changes depend on a seed that differs
// for every site and every browser session, so sites can't match the same person across
// sites or between sessions, while one site sees steady values for as long as the browser
// stays open.
//
// Runs in the page's own world before any of its scripts (a WebKit user script, from
// src/pagescripts.rs). The browser puts its settings in place of __CONFIG__:
// { key: a random key for this session, paused: sites with the shield off }.
;(function (config) {
  'use strict'

  // The site of the top page: frames get the same seed as the page around them.
  var top = location.hostname
  try {
    var origins = location.ancestorOrigins
    if (origins && origins.length) top = new URL(origins[origins.length - 1]).hostname
  } catch (e) {}
  if (!top) return
  for (var p = 0; p < config.paused.length; p++) {
    var site = config.paused[p]
    if (top === site || top.slice(-site.length - 1) === '.' + site) return
  }

  // FNV-1a over the session key and the site.
  var text = config.key + '|' + top
  var hash = 0x811c9dc5
  for (var c = 0; c < text.length; c++) {
    hash ^= text.charCodeAt(c)
    hash = Math.imul(hash, 0x01000193)
  }
  farble(hash >>> 0)

  function farble(seed) {
    var nativeToString = Function.prototype.toString
    // Replaced functions and getters, with the source text the originals show.
    var natives = new WeakMap()

    function mix(value) {
      var n = Math.imul(value ^ (value >>> 16), 0x45d9f3b)
      n = Math.imul(n ^ (n >>> 16), 0x45d9f3b)
      return (n ^ (n >>> 16)) >>> 0
    }

    // Swaps a method for one that looks like the original: same name, length and source text.
    function patch(target, name, make) {
      if (!target) return
      var descriptor = Object.getOwnPropertyDescriptor(target, name)
      var original = descriptor && descriptor.value
      if (!descriptor || typeof original !== 'function') return
      var wrapped = make(original)
      // A method definition can't be called with `new` and has no prototype, like a native one.
      var replacement = {
        [name]() {
          return wrapped.apply(this, arguments)
        },
      }[name]
      Object.defineProperty(replacement, 'length', { value: original.length })
      natives.set(replacement, nativeToString.call(original))
      descriptor.value = replacement
      Object.defineProperty(target, name, descriptor)
    }

    function patchGetter(target, name, make) {
      if (!target) return
      var descriptor = Object.getOwnPropertyDescriptor(target, name)
      var original = descriptor && descriptor.get
      if (!descriptor || !original) return
      var wrapped = make(original)
      var getter = Object.getOwnPropertyDescriptor(
        {
          get [name]() {
            return wrapped.call(this)
          },
        },
        name,
      ).get
      natives.set(getter, nativeToString.call(original))
      descriptor.get = getter
      Object.defineProperty(target, name, descriptor)
    }

    patch(Function.prototype, 'toString', function (original) {
      return function () {
        var native = natives.get(this)
        return native !== undefined ? native : original.call(this)
      }
    })

    // ── Canvas ───────────────────────────────────────────────────────────────

    // Flips the lowest bit of one colour in about one opaque pixel out of fifty.
    function noise(data) {
      for (var i = 0; i + 3 < data.length; i += 4) {
        if (data[i + 3] !== 255) continue
        var h = mix(seed ^ Math.imul(i, 0x27d4eb2d) ^ (data[i] << 16) ^ (data[i + 1] << 8) ^ data[i + 2])
        if (h % 50 === 0) data[i + ((h >>> 8) % 3)] ^= 1
      }
    }

    var getImageData2D = CanvasRenderingContext2D.prototype.getImageData
    var drawImage2D = CanvasRenderingContext2D.prototype.drawImage
    var putImageData2D = CanvasRenderingContext2D.prototype.putImageData

    // A copy of a canvas with the noise added, or null to read the original (and get its errors).
    function noisyCopy(canvas) {
      var width = canvas.width
      var height = canvas.height
      if (!width || !height || width * height > 16000000) return null
      try {
        var copy = document.createElement('canvas')
        copy.width = width
        copy.height = height
        var context = copy.getContext('2d')
        if (!context) return null
        drawImage2D.call(context, canvas, 0, 0)
        var image = getImageData2D.call(context, 0, 0, width, height)
        noise(image.data)
        putImageData2D.call(context, image, 0, 0)
        return copy
      } catch (e) {
        return null
      }
    }

    patch(HTMLCanvasElement.prototype, 'toDataURL', function (original) {
      return function () {
        return original.apply(noisyCopy(this) || this, arguments)
      }
    })
    patch(HTMLCanvasElement.prototype, 'toBlob', function (original) {
      return function () {
        return original.apply(noisyCopy(this) || this, arguments)
      }
    })
    var contexts = [CanvasRenderingContext2D.prototype]
    if (window.OffscreenCanvasRenderingContext2D) contexts.push(window.OffscreenCanvasRenderingContext2D.prototype)
    contexts.forEach(function (proto) {
      patch(proto, 'getImageData', function (original) {
        return function () {
          var image = original.apply(this, arguments)
          noise(image.data)
          return image
        }
      })
    })

    // ── WebGL ────────────────────────────────────────────────────────────────

    var UNMASKED_VENDOR = 0x9245
    var UNMASKED_RENDERER = 0x9246
    var MAKER = /(Intel|AMD|ATI|Radeon|NVIDIA|GeForce|Mali|Adreno|Apple|Qualcomm|PowerVR|VMware|llvmpipe)/i
    ;[window.WebGLRenderingContext, window.WebGL2RenderingContext].forEach(function (type) {
      if (!type) return
      patch(type.prototype, 'readPixels', function (original) {
        return function () {
          var result = original.apply(this, arguments)
          var pixels = arguments[6]
          if (pixels instanceof Uint8Array) noise(pixels)
          return result
        }
      })
      // Keep the GPU maker (sites pick graphics quality by it), drop the exact model and driver.
      patch(type.prototype, 'getParameter', function (original) {
        return function (parameter) {
          var value = original.call(this, parameter)
          if (typeof value !== 'string') return value
          var maker = value.match(MAKER)
          if (parameter === UNMASKED_RENDERER) return maker ? maker[1] + ' GPU' : 'WebKit WebGL'
          if (parameter === UNMASKED_VENDOR) return maker ? maker[1] : 'WebKit'
          return value
        }
      })
    })

    // ── Audio ────────────────────────────────────────────────────────────────

    // Off by at most a hundredth of a percent: far below hearing, enough to change a hash.
    var gain = 1 + (((seed % 997) + 1) / 997) * 1e-4 * (seed & 1 ? 1 : -1)
    var scaled = new WeakSet()
    if (window.AudioBuffer) {
      patch(AudioBuffer.prototype, 'getChannelData', function (original) {
        return function () {
          var data = original.apply(this, arguments)
          if (!scaled.has(data)) {
            scaled.add(data)
            for (var i = 0; i < data.length; i++) data[i] *= gain
          }
          return data
        }
      })
      patch(AudioBuffer.prototype, 'copyFromChannel', function (original) {
        return function () {
          var result = original.apply(this, arguments)
          var destination = arguments[0]
          if (destination instanceof Float32Array) for (var i = 0; i < destination.length; i++) destination[i] *= gain
          return result
        }
      })
    }
    if (window.AnalyserNode) {
      patch(AnalyserNode.prototype, 'getFloatFrequencyData', function (original) {
        return function () {
          var result = original.apply(this, arguments)
          var array = arguments[0]
          if (array instanceof Float32Array) for (var i = 0; i < array.length; i++) array[i] *= gain
          return result
        }
      })
    }

    // ── Hardware ─────────────────────────────────────────────────────────────

    // Processor counts in a few common steps instead of the exact number.
    patchGetter(Navigator.prototype, 'hardwareConcurrency', function (original) {
      return function () {
        var cores = original.call(this)
        return cores >= 8 ? 8 : cores >= 4 ? 4 : cores
      }
    })
  }
})(__CONFIG__)
