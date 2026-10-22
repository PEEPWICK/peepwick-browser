import { useEffect, useRef, useState } from 'react'
import { ChevronDown, ChevronUp, X } from 'lucide-react'
import type { MessageKey } from '@shared/i18n'
import type { PermissionPrompt, TabState } from '@shared/types'
import { api } from '../api'
import { useT } from '../store'
import { Button, IconButton } from '../ui'

export function PermissionBar({ prompt }: { prompt: PermissionPrompt }) {
  const t = useT()
  const [remember, setRemember] = useState(true)
  const what = prompt.kinds.map((kind) => t(`perm.${kind}` as MessageKey)).join(t('perm.and'))
  let origin = prompt.origin
  try {
    origin = new URL(prompt.origin).host
  } catch {
    // Keep the raw origin.
  }
  return (
    <div className="bar permission-bar" role="alertdialog" aria-live="polite">
      <span className="bar-dot" aria-hidden />
      <p className="bar-text">{t('perm.ask', { origin, what })}</p>
      <label className="bar-check">
        <input type="checkbox" checked={remember} onChange={(event) => setRemember(event.target.checked)} />
        <span>{t('perm.remember')}</span>
      </label>
      <Button variant="quiet" onClick={() => api.permission(prompt.id, false, remember)}>
        {t('perm.deny')}
      </Button>
      <Button variant="primary" onClick={() => api.permission(prompt.id, true, remember)}>
        {t('perm.allow')}
      </Button>
    </div>
  )
}

export function FindBar({ tab, onClose }: { tab: TabState; onClose: () => void }) {
  const t = useT()
  const input = useRef<HTMLInputElement>(null)
  const [text, setText] = useState(tab.find?.text ?? '')

  useEffect(() => {
    const focus = () => {
      input.current?.focus()
      input.current?.select()
    }
    focus()
    window.addEventListener('pw-find-focus', focus)
    return () => window.removeEventListener('pw-find-focus', focus)
  }, [])

  const close = () => {
    api.stopFind()
    onClose()
  }

  const matches = tab.find?.matches ?? 0
  const active = tab.find?.active ?? 0
  return (
    <div className="bar find-bar">
      <input
        ref={input}
        className="find-input"
        value={text}
        placeholder={t('find.placeholder')}
        spellCheck={false}
        aria-label={t('find.placeholder')}
        onChange={(event) => {
          setText(event.target.value)
          api.find(event.target.value, true, false)
        }}
        onKeyDown={(event) => {
          if (event.key === 'Enter') api.find(text, !event.shiftKey, true)
          if (event.key === 'Escape') close()
        }}
      />
      <span className="find-count mono">
        {text ? (matches > 0 ? t('find.count', { active, total: matches }) : t('find.none')) : ''}
      </span>
      <IconButton icon={ChevronUp} label={t('find.prev')} disabled={!matches} onClick={() => api.find(text, false, true)} />
      <IconButton icon={ChevronDown} label={t('find.next')} disabled={!matches} onClick={() => api.find(text, true, true)} />
      <IconButton icon={X} label={t('find.close')} onClick={close} />
    </div>
  )
}
