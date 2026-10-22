// Small building blocks shared across the UI.
import { useEffect, useState, type ButtonHTMLAttributes, type ReactNode } from 'react'
import {
  BookOpen,
  Briefcase,
  Circle,
  Code,
  Flame,
  FlaskConical,
  Globe,
  Heart,
  House,
  Music,
  Palette,
  Star,
  type LucideIcon,
  type LucideProps,
} from 'lucide-react'
import type { WorkspaceIcon } from '@shared/types'
import { displayHost } from '@shared/url'
import { api } from './api'

/** Lucide icons at the brand's 1.5 px stroke. */
export function Icon({ icon: Glyph, size = 16, ...rest }: { icon: LucideIcon } & LucideProps) {
  return <Glyph size={size} strokeWidth={1.5} absoluteStrokeWidth {...rest} />
}

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  icon: LucideIcon
  label: string
  size?: number
  active?: boolean
}

export function IconButton({ icon, label, size = 16, active, className, ...rest }: IconButtonProps) {
  return (
    <button
      type="button"
      className={`icon-button ${active ? 'is-active' : ''} ${className ?? ''}`}
      aria-label={label}
      title={label}
      {...rest}
    >
      <Icon icon={icon} size={size} />
    </button>
  )
}

export const WORKSPACE_GLYPHS: Record<WorkspaceIcon, LucideIcon> = {
  circle: Circle,
  briefcase: Briefcase,
  house: House,
  book: BookOpen,
  code: Code,
  palette: Palette,
  music: Music,
  flask: FlaskConical,
  globe: Globe,
  heart: Heart,
  star: Star,
  flame: Flame,
}

export function Favicon({ src, url, size = 16 }: { src: string | null | undefined; url?: string; size?: number }) {
  const [broken, setBroken] = useState(false)
  useEffect(() => setBroken(false), [src])
  if (src && !broken) {
    return <img className="favicon" src={src} width={size} height={size} alt="" draggable={false} onError={() => setBroken(true)} />
  }
  const letter = (url ? displayHost(url) : '').charAt(0).toLocaleUpperCase('tr') || '·'
  return (
    <span className="favicon favicon-letter" style={{ width: size, height: size, fontSize: Math.round(size * 0.62) }}>
      {letter}
    </span>
  )
}

export function Toggle({ checked, onChange, label }: { checked: boolean; onChange: (value: boolean) => void; label: string }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className={`toggle ${checked ? 'is-on' : ''}`}
      onClick={() => onChange(!checked)}
    >
      <span className="toggle-knob" />
    </button>
  )
}

export function Segmented<T extends string | number>({
  value,
  options,
  onChange,
  label,
}: {
  value: T
  options: { value: T; label: ReactNode }[]
  onChange: (value: T) => void
  label: string
}) {
  return (
    <div className="segmented" role="radiogroup" aria-label={label}>
      {options.map((option) => (
        <button
          key={String(option.value)}
          type="button"
          role="radio"
          aria-checked={option.value === value}
          className={option.value === value ? 'is-active' : ''}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  )
}

export function Button({
  children,
  variant = 'secondary',
  className,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: 'primary' | 'secondary' | 'quiet' | 'accent' }) {
  return (
    <button type="button" className={`button button-${variant} ${className ?? ''}`} {...rest}>
      {children}
    </button>
  )
}

export function remaining(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000))
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = total % 60
  const mm = String(m).padStart(2, '0')
  const ss = String(s).padStart(2, '0')
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`
}

export function useNow(interval = 1000, enabled = true): number {
  const [now, setNow] = useState(Date.now())
  useEffect(() => {
    if (!enabled) return
    const id = setInterval(() => setNow(Date.now()), interval)
    return () => clearInterval(id)
  }, [interval, enabled])
  return now
}

let overlays = 0

/** Raises the UI above the pages while something floating is open. */
export function useOverlay(open: boolean): void {
  useEffect(() => {
    if (!open) return
    overlays++
    if (overlays === 1) api.setOverlay(true)
    return () => {
      overlays--
      if (overlays === 0) api.setOverlay(false)
    }
  }, [open])
}
