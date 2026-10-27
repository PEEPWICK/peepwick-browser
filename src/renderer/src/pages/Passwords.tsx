// The password manager (peepwick://passwords): saved logins by site, a local check
// for weak and reused passwords, adding, editing, import and export. Passwords are
// fetched one at a time, only when the person asks to see or copy them.
import { useEffect, useMemo, useState } from 'react'
import { Copy, Download, Eye, EyeOff, KeyRound, Pencil, Plus, Search, ShieldCheck, TriangleAlert, Trash2, Upload, X } from 'lucide-react'
import type { LoginEntry, PasswordStatus } from '@shared/types'
import { displayHost } from '@shared/url'
import { api, call } from '../api'
import { useUI } from '../context'
import { useT } from '../store'
import { Button, Favicon, Icon, IconButton, faviconFor } from '../ui'

type Filter = 'all' | 'weak' | 'reused'

function LoginRow({ login }: { login: LoginEntry }) {
  const { openPanel, showToast } = useUI()
  const t = useT()
  const [shown, setShown] = useState<string | null>(null)
  return (
    <li className="login-row">
      <Favicon src={faviconFor(login.origin)} url={login.origin} size={18} />
      <button type="button" className="login-site" title={login.origin} onClick={() => api.openURL(login.origin, true)}>
        {displayHost(login.origin)}
      </button>
      <span className="login-user">{login.username || <span className="muted">{t('autofill.noUsername')}</span>}</span>
      <span className="login-password mono">{shown ?? '••••••••••'}</span>
      <span className="login-flags">
        {login.weak && <span className="label login-flag">{t('passwords.weak')}</span>}
        {login.reused && <span className="label login-flag">{t('passwords.reused')}</span>}
      </span>
      <span className="login-actions">
        <IconButton
          icon={shown === null ? Eye : EyeOff}
          label={shown === null ? t('passwords.show') : t('passwords.hide')}
          onClick={() => (shown === null ? void call.passwordReveal(login.id).then((value) => setShown(value ?? '')) : setShown(null))}
        />
        <IconButton
          icon={Copy}
          label={t('passwords.copy')}
          onClick={() => {
            api.passwordCopy(login.id)
            showToast({ text: t('passwords.copied') })
          }}
        />
        <IconButton icon={Pencil} label={t('passwords.edit')} onClick={() => openPanel({ kind: 'password-edit', id: login.id })} />
        <IconButton
          icon={Trash2}
          label={t('passwords.delete')}
          onClick={() => {
            api.passwordsRemove([login.id])
            showToast({ text: t('passwords.deleted', { n: 1 }), action: { label: t('bookmarks.undo'), run: () => api.passwordsUndo() } })
          }}
        />
      </span>
    </li>
  )
}

export function PasswordsPage() {
  const { openPanel, ask, showToast } = useUI()
  const t = useT()
  const [logins, setLogins] = useState<LoginEntry[] | null>(null)
  const [status, setStatus] = useState<PasswordStatus | null>(null)
  const [text, setText] = useState('')
  const [filter, setFilter] = useState<Filter>('all')

  useEffect(() => {
    const load = () => {
      void call.passwordsList().then(setLogins)
      void call.passwordsStatus().then(setStatus)
    }
    load()
    window.addEventListener('pw-passwords-changed', load)
    return () => window.removeEventListener('pw-passwords-changed', load)
  }, [])

  const weak = logins?.filter((l) => l.weak).length ?? 0
  const reused = logins?.filter((l) => l.reused).length ?? 0
  const shown = useMemo(() => {
    const needle = text.trim().toLocaleLowerCase()
    return (logins ?? []).filter(
      (l) =>
        (filter === 'all' || (filter === 'weak' ? l.weak : l.reused)) &&
        (!needle || l.origin.toLowerCase().includes(needle) || l.username.toLocaleLowerCase().includes(needle)),
    )
  }, [logins, text, filter])

  const exportAll = () =>
    void ask({ title: t('passwords.exportConfirm'), body: t('passwords.exportBody'), confirm: t('passwords.exportRun'), cancel: t('common.cancel'), danger: true }).then(
      (ok) => ok && void call.passwordsExport().then((done) => done && showToast({ text: t('passwords.exported') })),
    )

  return (
    <div className="page list-page">
      <div className="list-page-inner">
        <header className="list-head">
          <h1 className="display small">{t('passwords.title')}</h1>
          <div className="list-head-actions">
            <Button variant="quiet" onClick={() => openPanel({ kind: 'import', focus: 'passwords' })}>
              <Icon icon={Download} size={15} /> {t('passwords.import')}
            </Button>
            <Button variant="quiet" disabled={!logins || logins.length === 0} onClick={exportAll}>
              <Icon icon={Upload} size={15} /> {t('passwords.export')}
            </Button>
            <Button onClick={() => openPanel({ kind: 'password-edit' })}>
              <Icon icon={Plus} size={15} /> {t('passwords.add')}
            </Button>
          </div>
        </header>

        {status && (
          <p className={`passwords-storage ${status.backend === 'basic_text' ? 'is-weak' : ''}`}>
            <Icon icon={status.backend === 'basic_text' ? TriangleAlert : KeyRound} size={15} />
            {status.backend === 'basic_text' ? t('passwords.weakStorage') : t('passwords.protected')}
          </p>
        )}

        {logins && logins.length > 0 && (
          <div className="passwords-check">
            <Icon icon={weak + reused > 0 ? TriangleAlert : ShieldCheck} size={18} />
            <div>
              <div className="setting-title">{t('passwords.checkup')}</div>
              <div className="setting-hint">{weak + reused > 0 ? t('passwords.checkupBody', { weak, reused }) : t('passwords.checkupGood')}</div>
            </div>
            <div className="segmented passwords-filter" role="radiogroup">
              {(['all', 'weak', 'reused'] as Filter[]).map((value) => (
                <button key={value} type="button" role="radio" aria-checked={filter === value} className={filter === value ? 'is-active' : ''} onClick={() => setFilter(value)}>
                  {t(`passwords.${value}`)} {value === 'weak' ? weak : value === 'reused' ? reused : logins.length}
                </button>
              ))}
            </div>
          </div>
        )}

        <label className="search-field">
          <Icon icon={Search} size={16} />
          <input value={text} placeholder={t('passwords.search')} spellCheck={false} onChange={(event) => setText(event.target.value)} autoFocus />
          {text && <IconButton icon={X} label={t('common.close')} onClick={() => setText('')} />}
        </label>

        {logins && logins.length === 0 && <p className="muted list-empty">{t('passwords.empty')}</p>}
        {logins && logins.length > 0 && shown.length === 0 && <p className="muted list-empty">{t('passwords.noResults')}</p>}

        {shown.length > 0 && (
          <ul className="login-list">
            {shown.map((login) => (
              <LoginRow key={login.id} login={login} />
            ))}
          </ul>
        )}

        {status && status.never.length > 0 && (
          <section className="never-list">
            <h2 className="label">{t('passwords.never')}</h2>
            <p className="setting-hint">{t('passwords.neverHint')}</p>
            <ul>
              {status.never.map((origin) => (
                <li key={origin}>
                  <Favicon src={faviconFor(origin)} url={origin} size={16} />
                  <span className="mono">{displayHost(origin)}</span>
                  <IconButton icon={X} label={t('settings.remove')} onClick={() => api.passwordNeverRemove(origin)} />
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
    </div>
  )
}
