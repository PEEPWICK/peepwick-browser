// Encrypted backup: choosing what goes in and the password, and restoring a file
// (password, what to restore, what happened). The work happens in the main process
// (backup.ts); the password never leaves this window except to it.
import { useEffect, useRef, useState } from 'react'
import { X } from 'lucide-react'
import type { MessageKey } from '@shared/i18n'
import { BACKUP_CATEGORIES, BACKUP_DEFAULTS, type BackupCategory, type BackupCounts, type BackupSummary, type RestoreResult } from '@shared/types'
import { api, call } from '../api'
import { useUI } from '../context'
import { useSnapshot, useT } from '../store'
import { Button, IconButton } from '../ui'
import { Modal } from './BookmarkDialogs'

function Categories({
  counts,
  chosen,
  onChange,
}: {
  counts: BackupCounts
  chosen: BackupCategory[]
  onChange: (chosen: BackupCategory[]) => void
}) {
  const t = useT()
  const toggle = (category: BackupCategory, on: boolean) => {
    let next = on ? [...chosen, category] : chosen.filter((c) => c !== category)
    // Tabs belong to spaces: one without the other doesn't restore sensibly.
    if (category === 'tabs' && on && !next.includes('workspaces')) next = [...next, 'workspaces']
    if (category === 'workspaces' && !on) next = next.filter((c) => c !== 'tabs')
    onChange(next)
  }
  return (
    <div className="clear-kinds backup-categories">
      {BACKUP_CATEGORIES.filter((c) => counts[c] !== undefined).map((category) => (
        <label key={category} className="clear-kind">
          <input
            type="checkbox"
            checked={chosen.includes(category) && !!counts[category]}
            disabled={!counts[category]}
            onChange={(event) => toggle(category, event.target.checked)}
          />
          <span>
            <span className="clear-kind-title">{t(`backup.cat.${category}` as MessageKey)}</span>
            <span className="clear-kind-hint muted">
              {t(`backup.count.${category}` as MessageKey, { n: counts[category] ?? 0 })}
            </span>
          </span>
        </label>
      ))}
    </div>
  )
}

export function BackupExportDialog() {
  const { closePanel, showToast } = useUI()
  const t = useT()
  const [counts, setCounts] = useState<BackupCounts | null>(null)
  const [chosen, setChosen] = useState<BackupCategory[]>(BACKUP_DEFAULTS)
  const [password, setPassword] = useState('')
  const [repeat, setRepeat] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    void call.backupCounts().then(setCounts)
  }, [])

  const save = async () => {
    if (password.length < 8) return setError(t('backup.passwordShort'))
    if (password !== repeat) return setError(t('backup.passwordMismatch'))
    setBusy(true)
    const result = await call.backupExport(chosen, password)
    setBusy(false)
    if (result === 'cancelled') return
    closePanel()
    showToast({ text: result === 'saved' ? t('backup.saved') : t('backup.failed') })
  }

  return (
    <Modal label={t('backup.export')} width={480} onClose={closePanel}>
      <div className="modal-head">
        <h2>{t('backup.export')}</h2>
        <IconButton icon={X} label={t('common.close')} onClick={closePanel} />
      </div>
      <form
        className="modal-body"
        onSubmit={(event) => {
          event.preventDefault()
          void save()
        }}
      >
        <span className="label">{t('backup.what')}</span>
        {counts && <Categories counts={counts} chosen={chosen} onChange={setChosen} />}
        <span className="setting-hint">{t('backup.tabsNeedSpaces')}</span>
        <label className="field">
          <span className="label">{t('backup.password')}</span>
          <input
            className="text-input"
            type="password"
            autoComplete="new-password"
            value={password}
            onChange={(event) => {
              setPassword(event.target.value)
              setError('')
            }}
          />
        </label>
        <label className="field">
          <span className="label">{t('backup.passwordRepeat')}</span>
          <input
            className="text-input"
            type="password"
            autoComplete="new-password"
            value={repeat}
            onChange={(event) => {
              setRepeat(event.target.value)
              setError('')
            }}
          />
          {error ? <span className="field-error">{error}</span> : <span className="setting-hint">{t('backup.passwordHint')}</span>}
        </label>
        <div className="panel-actions">
          <Button variant="quiet" onClick={closePanel}>
            {t('common.cancel')}
          </Button>
          <Button variant="primary" type="submit" disabled={busy || chosen.length === 0 || !password}>
            {busy ? t('backup.saving') : t('backup.create')}
          </Button>
        </div>
      </form>
    </Modal>
  )
}

/** Restoring: unlock with the password, choose what to bring in, see what happened. */
export function BackupRestoreDialog({ token, name }: { token: string; name: string }) {
  const { closePanel } = useUI()
  const { settings } = useSnapshot()
  const t = useT()
  const [password, setPassword] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [summary, setSummary] = useState<BackupSummary | null>(null)
  const [chosen, setChosen] = useState<BackupCategory[]>([])
  const [extensions, setExtensions] = useState<string[]>([])
  const [result, setResult] = useState<RestoreResult | null>(null)
  const finished = useRef(false)

  // Closing early lets the main process forget the opened file.
  useEffect(
    () => () => {
      if (!finished.current) api.backupCancel(token)
    },
    [token],
  )

  const unlock = async () => {
    setBusy(true)
    const answer = await call.backupUnlock(token, password)
    setBusy(false)
    if ('error' in answer) {
      setError(answer.error === 'password' ? t('backup.wrongPassword') : answer.error === 'expired' ? t('backup.expired') : t('backup.notBackup'))
      return
    }
    setPassword('')
    setSummary(answer)
    setChosen(BACKUP_CATEGORIES.filter((c) => (answer.counts[c] ?? 0) > 0))
    setExtensions(answer.extensions.map((e) => e.id))
  }

  const restore = async () => {
    setBusy(true)
    const done = await call.backupRestore(token, chosen, extensions)
    finished.current = true
    setBusy(false)
    if (done) setResult(done)
    else setError(t('backup.expired'))
  }

  const date = (time: number) => new Intl.DateTimeFormat(settings.language === 'tr' ? 'tr-TR' : 'en-US', { dateStyle: 'long', timeStyle: 'short' }).format(time)

  let body: React.ReactNode
  if (result) {
    const lines = [
      result.settings && t('backup.result.settings'),
      result.bookmarks > 0 && t('backup.result.bookmarks', { n: result.bookmarks }),
      result.workspaces > 0 && t('backup.result.workspaces', { n: result.workspaces }),
      result.tabs > 0 && t('backup.result.tabs', { n: result.tabs }),
      result.focusDays > 0 && t('backup.result.focus', { n: result.focusDays }),
      result.permissions > 0 && t('backup.result.permissions', { n: result.permissions }),
      result.extensions > 0 && t('backup.result.extensions', { n: result.extensions }),
      result.history > 0 && t('backup.result.history', { n: result.history }),
    ].filter(Boolean) as string[]
    body = (
      <>
        {lines.length > 0 ? (
          <ul className="restore-result">
            {lines.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
        ) : (
          <p className="setting-hint">{t('backup.result.none')}</p>
        )}
        <div className="panel-actions">
          <Button variant="primary" onClick={closePanel}>
            {t('bookmarks.done')}
          </Button>
        </div>
      </>
    )
  } else if (summary) {
    body = (
      <>
        <p className="setting-hint">{t('backup.restoreFrom', { date: date(summary.createdAt), version: summary.version })}</p>
        <Categories counts={summary.counts} chosen={chosen} onChange={setChosen} />
        {chosen.includes('extensions') && summary.extensions.length > 0 && (
          <div className="field">
            <span className="setting-hint">{t('backup.reinstall')}</span>
            {summary.extensions.map((extension) => (
              <label key={extension.id} className="check">
                <input
                  type="checkbox"
                  checked={extensions.includes(extension.id)}
                  onChange={(event) => setExtensions((ids) => (event.target.checked ? [...ids, extension.id] : ids.filter((id) => id !== extension.id)))}
                />
                <span>{extension.name}</span>
              </label>
            ))}
          </div>
        )}
        <p className="setting-hint">{t('backup.restoreMerge')}</p>
        {error && <span className="field-error">{error}</span>}
        <div className="panel-actions">
          <Button variant="quiet" onClick={closePanel}>
            {t('common.cancel')}
          </Button>
          <Button variant="primary" disabled={busy || chosen.length === 0} onClick={() => void restore()}>
            {busy ? t('backup.restoring') : t('backup.restore')}
          </Button>
        </div>
      </>
    )
  } else {
    body = (
      <form
        className="backup-unlock"
        onSubmit={(event) => {
          event.preventDefault()
          void unlock()
        }}
      >
        <p className="setting-hint">{t('backup.unlockBody', { name })}</p>
        <label className="field">
          <span className="label">{t('backup.password')}</span>
          <input
            className="text-input"
            type="password"
            autoComplete="current-password"
            autoFocus
            value={password}
            onChange={(event) => {
              setPassword(event.target.value)
              setError('')
            }}
          />
          {error && <span className="field-error">{error}</span>}
        </label>
        <div className="panel-actions">
          <Button variant="quiet" onClick={closePanel}>
            {t('common.cancel')}
          </Button>
          <Button variant="primary" type="submit" disabled={busy || !password}>
            {busy ? t('backup.unlocking') : t('backup.unlock')}
          </Button>
        </div>
      </form>
    )
  }

  const heading = result ? t('backup.restored') : summary ? t('backup.restoreTitle') : t('backup.unlockTitle')
  return (
    <Modal label={heading} width={480} onClose={closePanel}>
      <div className="modal-head">
        <h2>{heading}</h2>
        <IconButton icon={X} label={t('common.close')} onClick={closePanel} />
      </div>
      <div className="modal-body">{body}</div>
    </Modal>
  )
}
