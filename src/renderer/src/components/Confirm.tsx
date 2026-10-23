// Questions the main process asks (adding an extension, opening many bookmarks,
// quitting during downloads). Drawn here instead of system message boxes so they
// follow the browser's theme.
import { useEffect, useRef } from 'react'
import { X } from 'lucide-react'
import type { ConfirmModel } from '@shared/types'
import { api } from '../api'
import { useUI } from '../context'
import { useT } from '../store'
import { Button, IconButton } from '../ui'
import { Modal } from './BookmarkDialogs'

export function ConfirmDialog({ dialog }: { dialog: ConfirmModel }) {
  const { closePanel } = useUI()
  const t = useT()
  const answered = useRef(false)

  const reply = (ok: boolean) => {
    if (!answered.current) {
      answered.current = true
      api.confirmReply(dialog.id, ok)
    }
    closePanel()
  }

  // Replaced by another panel or the window closing: that is a no.
  useEffect(
    () => () => {
      if (!answered.current) api.confirmReply(dialog.id, false)
    },
    [dialog.id],
  )

  return (
    <Modal label={dialog.title} width={420} onClose={() => reply(false)}>
      <div className="modal-head">
        <div className="confirm-title">
          {dialog.icon && <img src={dialog.icon} width={24} height={24} alt="" draggable={false} />}
          <h2>{dialog.title}</h2>
        </div>
        <IconButton icon={X} label={t('common.close')} onClick={() => reply(false)} />
      </div>
      <div className="modal-body">
        {dialog.body && <p className="confirm-body">{dialog.body}</p>}
        {dialog.lines && dialog.lines.length > 0 && (
          <ul className="confirm-lines">
            {dialog.lines.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
        )}
        <div className="panel-actions">
          <Button variant="quiet" autoFocus={dialog.danger} onClick={() => reply(false)}>
            {dialog.cancel}
          </Button>
          <Button variant="primary" autoFocus={!dialog.danger} onClick={() => reply(true)}>
            {dialog.confirm}
          </Button>
        </div>
      </div>
    </Modal>
  )
}
