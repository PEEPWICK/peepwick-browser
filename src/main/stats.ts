import type { Stats } from '@shared/types'
import { userPath } from './paths'
import { JsonFile } from './store'

interface Day {
  blocked: number
  focus: number
}

interface StatsFile {
  days: Record<string, Day>
  blockedTotal: number
}

const file = new JsonFile<StatsFile>(userPath('stats.json'), 5000)
let data: StatsFile = { days: {}, blockedTotal: 0 }

export function loadStats(): void {
  const saved = file.read()
  if (saved && typeof saved === 'object' && saved.days) data = saved
  prune()
}

export function dayKey(date = new Date()): string {
  const y = date.getFullYear()
  const m = String(date.getMonth() + 1).padStart(2, '0')
  const d = String(date.getDate()).padStart(2, '0')
  return `${y}-${m}-${d}`
}

function today(): Day {
  const key = dayKey()
  return (data.days[key] ??= { blocked: 0, focus: 0 })
}

export function addBlocked(count = 1): void {
  today().blocked += count
  data.blockedTotal += count
  file.schedule(data)
}

export function addFocusMinutes(minutes: number): void {
  if (minutes <= 0) return
  today().focus += minutes
  file.schedule(data)
}

function lastDays(n: number): Day[] {
  const out: Day[] = []
  const date = new Date()
  for (let i = 0; i < n; i++) {
    out.push(data.days[dayKey(date)] ?? { blocked: 0, focus: 0 })
    date.setDate(date.getDate() - 1)
  }
  return out
}

export function stats(): Stats {
  const week = lastDays(7)
  const day = week[0]
  return {
    blockedToday: day.blocked,
    blockedWeek: week.reduce((sum, d) => sum + d.blocked, 0),
    blockedTotal: data.blockedTotal,
    focusMinutesToday: Math.round(day.focus),
    focusMinutesWeek: Math.round(week.reduce((sum, d) => sum + d.focus, 0)),
  }
}

function prune(): void {
  const cutoff = new Date()
  cutoff.setDate(cutoff.getDate() - 90)
  const limit = dayKey(cutoff)
  for (const key of Object.keys(data.days)) if (key < limit) delete data.days[key]
}

export function flushStats(): void {
  file.flush()
}

export interface StatsData {
  days: Record<string, { blocked: number; focus: number }>
  blockedTotal: number
}

export function exportStats(): StatsData {
  return JSON.parse(JSON.stringify(data)) as StatsData
}

/** Restoring a backup: keeps the larger count for each day, so restoring twice adds nothing. */
export function mergeStats(incoming: StatsData): number {
  let days = 0
  for (const [key, day] of Object.entries(incoming.days ?? {})) {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(key) || typeof day?.blocked !== 'number' || typeof day?.focus !== 'number') continue
    const current = (data.days[key] ??= { blocked: 0, focus: 0 })
    if (day.blocked > current.blocked || day.focus > current.focus) days++
    current.blocked = Math.max(current.blocked, day.blocked)
    current.focus = Math.max(current.focus, day.focus)
  }
  if (typeof incoming.blockedTotal === 'number') data.blockedTotal = Math.max(data.blockedTotal, incoming.blockedTotal)
  prune()
  file.schedule(data)
  return days
}
