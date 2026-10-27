// "Clear browsing data" (Ctrl+Shift+Delete): a time range and what to clear, with
// counts so people see what they are about to delete.
import { useEffect, useState } from 'react'
import { X } from 'lucide-react'
import type { MessageKey } from '@shared/i18n'
import type { ClearCounts, ClearKind, ClearRange } from '@shared/types'
import { call } from '../api'
import { useUI } from '../context'
import { useT } from '../store'
import { Button, IconButton } from '../ui'
import { Modal } from './BookmarkDialogs'

const RANGES: ClearRange[] = [1, 24, 168, 672, 0]
/** Cleared for all time whatever the range says (Electron has no range for them). */
const ALL_TIME: ClearKind[] = ['cookies', 'cache', 'permissions', 'passwords']

// Remembered while the browser runs, like Chrome's dialog.
let lastRange: ClearRange = 0
let lastKinds: ClearKind[] = ['history', 'cache']

export function ClearDataDialog() {
  const { closePanel, showToast } = useUI()
  const t = useT()
  const [range, setRange] = useState<ClearRange>(lastRange)
  const [kinds, setKinds] = useState<ClearKind[]>(lastKinds)
  const [counts, setCounts] = useState<ClearCounts | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    void call.clearDataCounts(range).then(setCounts)
  }, [range])

  const toggle = (kind: ClearKind, on: boolean) => setKinds((current) => (on ? [...current, kind] : current.filter((k) => k !== kind)))

  const hint = (kind: ClearKind): string => {
    const allTime = range !== 0 && ALL_TIME.includes(kind) ? ` · ${t('clear.allTime')}` : ''
    switch (kind) {
      case 'history':
        return counts ? t('clear.historyHint', { n: counts.history }) : ''
      case 'downloads':
        return counts ? t('clear.downloadsHint', { n: counts.downloads }) : ''
      case 'cookies':
        return counts ? t('clear.cookiesHint', { n: counts.cookies }) + allTime : ''
      case 'cache':
        return t('clear.cacheHint') + allTime
      case 'permissions':
        return counts ? t('clear.permissionsHint', { n: counts.permissions }) + allTime : ''
      case 'forms':
        return counts ? t('clear.formsHint', { n: counts.forms }) : ''
      case 'passwords':
        return counts ? t('clear.passwordsHint', { n: counts.passwords }) + allTime : ''
    }
  }

  const run = async () => {
    setBusy(true)
    lastRange = range
    lastKinds = kinds
    await call.clearData({ range, kinds })
    setBusy(false)
    closePanel()
    showToast({ text: t('clear.done') })
  }

  const order: ClearKind[] = ['history', 'downloads', 'cookies', 'cache', 'forms', 'permissions', 'passwords']
  return (
    <Modal label={t('clear.title')} width={480} onClose={closePanel}>
      <div className="modal-head">
        <h2>{t('clear.title')}</h2>
        <IconButton icon={X} label={t('common.close')} onClick={closePanel} />
      </div>
      <div className="modal-body">
        <label className="field">
          <span className="label">{t('clear.range')}</span>
          <select className="select" value={range} onChange={(event) => setRange(Number(event.target.value) as ClearRange)}>
            {RANGES.map((value) => (
              <option key={value} value={value}>
                {t(`clear.range.${value}` as MessageKey)}
              </option>
            ))}
          </select>
        </label>
        <div className="clear-kinds">
          {order.map((kind) => (
            <label key={kind} className="clear-kind">
              <input type="checkbox" checked={kinds.includes(kind)} onChange={(event) => toggle(kind, event.target.checked)} />
              <span>
                <span className="clear-kind-title">{t(`clear.${kind}` as MessageKey)}</span>
                <span className="clear-kind-hint muted">{hint(kind)}</span>
              </span>
            </label>
          ))}
        </div>
        <div className="panel-actions">
          <Button variant="quiet" onClick={closePanel}>
            {t('common.cancel')}
          </Button>
          <Button variant="primary" disabled={kinds.length === 0 || busy} onClick={() => void run()}>
            {busy ? t('clear.running') : t('clear.run')}
          </Button>
        </div>
      </div>
    </Modal>
  )
}
