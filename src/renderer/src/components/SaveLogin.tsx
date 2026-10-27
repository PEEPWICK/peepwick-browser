// "Save password for example.com?" after a sign-in. The password itself stays in
// the main process; the bar only shows the site and the username.
import { KeyRound, X } from 'lucide-react'
import type { SaveLoginPrompt } from '@shared/types'
import { api } from '../api'
import { useT } from '../store'
import { Button, Icon, IconButton } from '../ui'

export function SaveLoginBar({ prompt, onDone }: { prompt: SaveLoginPrompt; onDone: () => void }) {
  const t = useT()
  const answer = (choice: 'save' | 'never' | 'dismiss') => {
    api.saveLoginAnswer(prompt.id, choice)
    onDone()
  }
  return (
    <div className="bar save-login-bar" role="dialog" aria-label={t(prompt.update ? 'save.update' : 'save.title', { host: prompt.host })}>
      <Icon icon={KeyRound} size={16} className="accent" />
      <p className="bar-text">
        <strong>{t(prompt.update ? 'save.update' : 'save.title', { host: prompt.host })}</strong>
        <span className="muted mono"> {prompt.username ? t('save.user', { user: prompt.username }) : t('save.noUser')}</span>
      </p>
      <Button variant="quiet" onClick={() => answer('never')}>
        {t('save.never')}
      </Button>
      <Button variant="primary" onClick={() => answer('save')}>
        {prompt.update ? t('save.updateRun') : t('save.save')}
      </Button>
      <IconButton icon={X} label={t('save.dismiss')} onClick={() => answer('dismiss')} />
    </div>
  )
}
