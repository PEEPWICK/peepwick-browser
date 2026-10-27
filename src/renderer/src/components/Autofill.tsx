// Settings › Passwords and autofill, and the dialogs for one address or password.
import { useEffect, useState } from 'react'
import { KeyRound, MapPin, Pencil, Plus, Trash2, X } from 'lucide-react'
import type { MessageKey } from '@shared/i18n'
import { originOf } from '@shared/passwordcsv'
import type { Address } from '@shared/types'
import { api, call } from '../api'
import { useUI } from '../context'
import { useSnapshot, useT } from '../store'
import { Button, Icon, IconButton, SettingRow as Row, Toggle } from '../ui'
import { Modal } from './BookmarkDialogs'

const FIELDS: (keyof Omit<Address, 'id'>)[] = ['name', 'organization', 'street', 'city', 'state', 'postalCode', 'country', 'phone', 'email']

function useRefresh(event: string, load: () => void): void {
  useEffect(() => {
    load()
    window.addEventListener(event, load)
    return () => window.removeEventListener(event, load)
  }, [])
}

export function AutofillSection() {
  const { settings: s } = useSnapshot()
  const { openPanel, ask, showToast } = useUI()
  const t = useT()
  const [count, setCount] = useState(0)
  const [addresses, setAddresses] = useState<Address[]>([])
  const [entries, setEntries] = useState(0)
  const load = () => {
    void call.passwordsStatus().then((status) => setCount(status.count))
    void call.addressesList().then(setAddresses)
    void call.formEntryCount().then(setEntries)
  }
  useRefresh('pw-passwords-changed', load)
  useRefresh('pw-addresses-changed', load)

  return (
    <>
      <Row title={t('autofill.manager')} hint={t('autofill.managerHint', { n: count })}>
        <Button onClick={() => api.openPage('passwords')}>
          <Icon icon={KeyRound} size={15} /> {t('autofill.open')}
        </Button>
      </Row>
      <Row title={t('autofill.save')} hint={t('autofill.saveHint')}>
        <Toggle checked={s.passwordSave} label={t('autofill.save')} onChange={(passwordSave) => api.setSettings({ passwordSave })} />
      </Row>
      <Row title={t('autofill.fill')} hint={t('autofill.fillHint')}>
        <Toggle checked={s.passwordFill} label={t('autofill.fill')} onChange={(passwordFill) => api.setSettings({ passwordFill })} />
      </Row>
      <Row title={t('autofill.forms')} hint={t('autofill.formsHint')}>
        <Toggle checked={s.formFill} label={t('autofill.forms')} onChange={(formFill) => api.setSettings({ formFill })} />
      </Row>
      <div className="setting-block">
        <div className="setting-head">
          <div className="setting-title">{t('autofill.addresses')}</div>
          <Button variant="quiet" onClick={() => openPanel({ kind: 'address-edit' })}>
            <Icon icon={Plus} size={15} /> {t('autofill.addAddress')}
          </Button>
        </div>
        {addresses.length === 0 ? (
          <p className="setting-hint">{t('autofill.addressesEmpty')}</p>
        ) : (
          <ul className="address-list">
            {addresses.map((address) => (
              <li key={address.id}>
                <Icon icon={MapPin} size={15} className="muted" />
                <span className="address-text">
                  <span>{address.name || address.organization || address.email}</span>
                  <span className="muted">{[address.street, address.city, address.country].filter(Boolean).join(', ')}</span>
                </span>
                <IconButton icon={Pencil} label={t('autofill.editAddress')} onClick={() => openPanel({ kind: 'address-edit', id: address.id })} />
                <IconButton
                  icon={Trash2}
                  label={t('settings.remove')}
                  onClick={() => {
                    api.addressRemove(address.id)
                    setTimeout(load, 80)
                  }}
                />
              </li>
            ))}
          </ul>
        )}
      </div>
      <Row title={t('autofill.entries')} hint={t('autofill.entriesHint', { n: entries })}>
        <Button
          variant="quiet"
          disabled={entries === 0}
          onClick={() =>
            void ask({ title: t('autofill.entries'), body: t('autofill.entriesHint', { n: entries }), confirm: t('autofill.entriesClear'), cancel: t('common.cancel'), danger: true }).then(
              (ok) => {
                if (!ok) return
                api.formEntriesClear()
                showToast({ text: t('autofill.entriesCleared') })
                setTimeout(load, 80)
              },
            )
          }
        >
          {t('autofill.entriesClear')}
        </Button>
      </Row>
    </>
  )
}

export function AddressDialog({ id }: { id?: string }) {
  const { closePanel } = useUI()
  const t = useT()
  const [address, setAddress] = useState<Partial<Address>>({})
  useEffect(() => {
    if (id) void call.addressesList().then((list) => setAddress(list.find((a) => a.id === id) ?? {}))
  }, [id])
  const save = async () => {
    await call.addressSave({ ...address, id })
    window.dispatchEvent(new Event('pw-addresses-changed'))
    closePanel()
  }
  return (
    <Modal label={id ? t('autofill.editAddress') : t('autofill.addAddress')} width={520} onClose={closePanel}>
      <div className="modal-head">
        <h2>{id ? t('autofill.editAddress') : t('autofill.addAddress')}</h2>
        <IconButton icon={X} label={t('common.close')} onClick={closePanel} />
      </div>
      <form
        className="modal-body"
        onSubmit={(event) => {
          event.preventDefault()
          void save()
        }}
      >
        <div className="address-grid">
          {FIELDS.map((field, index) => (
            <label key={field} className={`field ${field === 'street' || field === 'name' ? 'is-wide' : ''}`}>
              <span className="label">{t(`address.${field}` as MessageKey)}</span>
              <input
                className="text-input"
                autoFocus={index === 0}
                value={address[field] ?? ''}
                type={field === 'email' ? 'email' : field === 'phone' ? 'tel' : 'text'}
                onChange={(event) => setAddress((a) => ({ ...a, [field]: event.target.value }))}
              />
            </label>
          ))}
        </div>
        <div className="panel-actions">
          <Button variant="quiet" onClick={closePanel}>
            {t('common.cancel')}
          </Button>
          <Button variant="primary" type="submit" disabled={FIELDS.every((f) => !address[f]?.trim())}>
            {t('passwords.save')}
          </Button>
        </div>
      </form>
    </Modal>
  )
}

export function PasswordDialog({ id }: { id?: string }) {
  const { closePanel } = useUI()
  const t = useT()
  const [site, setSite] = useState('')
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [note, setNote] = useState('')
  const [error, setError] = useState('')

  useEffect(() => {
    if (!id) return
    void call.passwordsList().then((list) => {
      const login = list.find((l) => l.id === id)
      if (!login) return
      setSite(login.origin)
      setUsername(login.username)
      setNote(login.note)
    })
    void call.passwordReveal(id).then((value) => setPassword(value ?? ''))
  }, [id])

  const save = async () => {
    if (!originOf(site)) return setError(t('passwords.invalidSite'))
    if (!password) return setError(t('passwords.needPassword'))
    const ok = id ? await call.passwordUpdate(id, { origin: site, username, password, note }) : !!(await call.passwordAdd({ origin: site, username, password, note }))
    if (ok) closePanel()
    else setError(t('passwords.invalidSite'))
  }

  return (
    <Modal label={id ? t('passwords.editTitle') : t('passwords.add')} width={480} onClose={closePanel}>
      <div className="modal-head">
        <h2>{id ? t('passwords.editTitle') : t('passwords.add')}</h2>
        <IconButton icon={X} label={t('common.close')} onClick={closePanel} />
      </div>
      <form
        className="modal-body"
        onSubmit={(event) => {
          event.preventDefault()
          void save()
        }}
      >
        <label className="field">
          <span className="label">{t('passwords.site')}</span>
          <input className="text-input mono" autoFocus={!id} value={site} placeholder="https://example.com" spellCheck={false} onChange={(e) => (setSite(e.target.value), setError(''))} />
        </label>
        <label className="field">
          <span className="label">{t('passwords.username')}</span>
          <input className="text-input" value={username} spellCheck={false} autoComplete="off" onChange={(e) => setUsername(e.target.value)} />
        </label>
        <label className="field">
          <span className="label">{t('passwords.password')}</span>
          <input className="text-input mono" value={password} spellCheck={false} autoComplete="off" onChange={(e) => (setPassword(e.target.value), setError(''))} />
        </label>
        <label className="field">
          <span className="label">{t('passwords.note')}</span>
          <textarea className="text-input note-input" value={note} onChange={(e) => setNote(e.target.value)} />
          {error && <span className="field-error">{error}</span>}
        </label>
        <div className="panel-actions">
          <Button variant="quiet" onClick={closePanel}>
            {t('common.cancel')}
          </Button>
          <Button variant="primary" type="submit">
            {t('passwords.save')}
          </Button>
        </div>
      </form>
    </Modal>
  )
}
