// The Double-Eye W symbol and the small brand marks, drawn from the geometry in
// brand book §08. Motion follows the book: "look" (pupils slide right and back in
// 300 ms) and "blink" (only for the opening and for loading, never repeated).
import { useEffect, useId, useRef, useState } from 'react'

export type EyeMode = 'rest' | 'look' | 'scan' | 'blink' | 'closed' | 'down'

interface SymbolProps {
  height?: number
  mode?: EyeMode
  className?: string
  title?: string
}

export function DoubleEye({ height = 20, mode = 'rest', className, title }: SymbolProps) {
  const id = useId().replace(/:/g, '')
  const small = height < 32
  const width = (height * 121) / 90
  return (
    <svg
      className={`pw-symbol pw-${mode} ${className ?? ''}`}
      width={width}
      height={height}
      viewBox="0 0 121 90"
      role={title ? 'img' : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
    >
      {small ? (
        <>
          <clipPath id={`c${id}`}>
            <polygon points="0,0 19,12 102,12 121,0 121,110 0,110" />
          </clipPath>
          <path
            d="M9.5,-20 V55 A25.5,25.5 0 0 0 60.5,55 V34 M60.5,55 A25.5,25.5 0 0 0 111.5,55 V-20"
            fill="none"
            stroke="currentColor"
            strokeWidth="19"
            clipPath={`url(#c${id})`}
          />
          <g className="pw-pupils">
            {mode === 'closed' ? (
              <path d="M21.5,51 H48.5 M72.5,51 H99.5" stroke="currentColor" strokeWidth="7" />
            ) : (
              <path d="M21.5,51 A13.5,13.5 0 1 0 48.5,51 A13.5,13.5 0 1 0 21.5,51 Z M72.5,51 A13.5,13.5 0 1 0 99.5,51 A13.5,13.5 0 1 0 72.5,51 Z" fill="currentColor" />
            )}
          </g>
        </>
      ) : (
        <>
          <clipPath id={`c${id}`}>
            <polygon points="0,0 15,15 106,15 121,0 121,110 0,110" />
          </clipPath>
          <path
            d="M7.5,-20 V56 A26.5,26.5 0 0 0 60.5,56 V30 M60.5,56 A26.5,26.5 0 0 0 113.5,56 V-20"
            fill="none"
            stroke="currentColor"
            strokeWidth="15"
            clipPath={`url(#c${id})`}
          />
          <g className="pw-pupils">
            {mode === 'closed' ? (
              <path d="M22,52 H46 M75,52 H99" stroke="currentColor" strokeWidth="5" />
            ) : (
              <path
                fillRule="evenodd"
                fill="currentColor"
                d="M22,51 A12,12 0 1 0 46,51 A12,12 0 1 0 22,51 Z M35.44,45.96 A3.6,3.6 0 1 0 42.64,45.96 A3.6,3.6 0 1 0 35.44,45.96 Z M75,51 A12,12 0 1 0 99,51 A12,12 0 1 0 75,51 Z M88.44,45.96 A3.6,3.6 0 1 0 95.64,45.96 A3.6,3.6 0 1 0 88.44,45.96 Z"
              />
            )}
          </g>
        </>
      )}
    </svg>
  )
}

/** The look mark (••): two dots, used once per surface as a signature. */
export function LookMark({ className }: { className?: string }) {
  return (
    <svg className={`pw-lookmark ${className ?? ''}`} width="14" height="6" viewBox="0 0 14 6" aria-hidden>
      <circle cx="3" cy="3" r="2.5" fill="currentColor" />
      <circle cx="11" cy="3" r="2.5" fill="currentColor" />
    </svg>
  )
}

/**
 * The crowd pattern: a regular dot grid with a single dot that looks back,
 * following the pointer. "Attention in a crowd."
 */
export function Crowd({ spacing = 24 }: { spacing?: number }) {
  const ref = useRef<HTMLDivElement>(null)
  const [pupil, setPupil] = useState({ x: 0, y: 0 })
  const [spot, setSpot] = useState({ x: 0, y: 0 })

  useEffect(() => {
    const el = ref.current
    if (!el) return
    const place = () => {
      const rect = el.getBoundingClientRect()
      const cols = Math.floor(rect.width / spacing)
      const rows = Math.floor(rect.height / spacing)
      setSpot({ x: Math.round(cols * 0.78) * spacing + spacing / 2, y: Math.round(rows * 0.24) * spacing + spacing / 2 })
    }
    place()
    const observer = new ResizeObserver(place)
    observer.observe(el)
    const onMove = (event: PointerEvent) => {
      const rect = el.getBoundingClientRect()
      const dx = event.clientX - (rect.left + spot.x)
      const dy = event.clientY - (rect.top + spot.y)
      const length = Math.hypot(dx, dy) || 1
      const reach = Math.min(2.6, length / 60)
      setPupil({ x: (dx / length) * reach, y: (dy / length) * reach })
    }
    window.addEventListener('pointermove', onMove)
    return () => {
      observer.disconnect()
      window.removeEventListener('pointermove', onMove)
    }
  }, [spacing, spot.x, spot.y])

  return (
    <div className="pw-crowd" ref={ref} style={{ ['--spacing' as string]: `${spacing}px` }} aria-hidden>
      <svg className="pw-crowd-eye" style={{ left: spot.x - 8, top: spot.y - 8 }} width="16" height="16" viewBox="0 0 16 16">
        <circle cx="8" cy="8" r="6" fill="var(--fitil)" />
        <circle cx={8 + pupil.x} cy={8 + pupil.y} r="2.6" fill="var(--gece)" />
      </svg>
    </div>
  )
}
