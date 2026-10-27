// Wording shared by the downloads page and the downloads panel.
import { formatBytes, type MessageKey } from '@shared/i18n'
import type { DownloadState, Lang } from '@shared/types'

/** "3 sn", "4 dk", "1 sa 5 dk" */
function duration(lang: Lang, seconds: number): string {
  const s = Math.max(1, Math.round(seconds))
  const units = lang === 'tr' ? { h: 'sa', m: 'dk', s: 'sn' } : { h: 'h', m: 'min', s: 's' }
  if (s < 60) return `${s} ${units.s}`
  if (s < 3600) return `${Math.round(s / 60)} ${units.m}`
  const h = Math.floor(s / 3600)
  const m = Math.round((s % 3600) / 60)
  return m > 0 ? `${h} ${units.h} ${m} ${units.m}` : `${h} ${units.h}`
}

/** One line under the file name: progress, speed and time left, or how it ended. */
export function downloadStatus(item: DownloadState, lang: Lang, t: (key: MessageKey, vars?: Record<string, string | number>) => string): string {
  const size = (bytes: number) => formatBytes(lang, bytes)
  const amount = item.total > 0 ? `${size(item.received)} / ${size(item.total)}` : size(item.received)
  switch (item.status) {
    case 'progressing': {
      const parts = [amount]
      if (item.speed && item.speed > 0) {
        parts.push(t('downloads.speed', { speed: size(item.speed) }))
        if (item.total > item.received) parts.push(t('downloads.left', { time: duration(lang, (item.total - item.received) / item.speed) }))
      }
      return parts.join(' · ')
    }
    case 'paused':
      return `${t('downloads.paused')} · ${amount}`
    case 'cancelled':
      return t('downloads.cancelled')
    case 'interrupted':
      return item.received > 0 ? `${t('downloads.failed')} · ${amount}` : t('downloads.failed')
    case 'completed':
      return item.missing ? t('downloads.missing') : size(item.total || item.received)
  }
}
