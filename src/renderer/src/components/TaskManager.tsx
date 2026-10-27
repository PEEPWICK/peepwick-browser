// The task manager (Shift+Esc): the browser's processes with memory and processor
// use, refreshed every two seconds. A tab's process can be ended from here.
import { useEffect, useState } from 'react'
import { Cpu, Globe, LayoutPanelTop, Puzzle, Server, SquareActivity, X } from 'lucide-react'
import { formatBytes, type MessageKey } from '@shared/i18n'
import type { TaskRow } from '@shared/types'
import { api, call } from '../api'
import { useUI } from '../context'
import { useSnapshot, useT } from '../store'
import { Button, Favicon, Icon, IconButton } from '../ui'
import { Modal } from './BookmarkDialogs'

const ICONS = { browser: SquareActivity, gpu: Cpu, ui: LayoutPanelTop, extension: Puzzle, service: Server, other: Globe, tab: Globe }

function useTasks(): TaskRow[] {
  const [rows, setRows] = useState<TaskRow[]>([])
  useEffect(() => {
    let alive = true
    const load = () => void call.tasks().then((next) => alive && setRows(next))
    load()
    const id = setInterval(load, 2000)
    return () => {
      alive = false
      clearInterval(id)
    }
  }, [])
  return rows
}

export function TaskManagerDialog() {
  const { closePanel } = useUI()
  const { settings } = useSnapshot()
  const t = useT()
  const rows = useTasks()
  const [selected, setSelected] = useState<number | null>(null)
  const chosen = rows.find((row) => row.pid === selected)
  const total = rows.reduce((sum, row) => sum + row.memory, 0)

  const name = (row: TaskRow): string => {
    if (row.kind === 'tab') return row.tabs[0].title
    if (row.kind === 'service') {
      const key = `task.service.${row.name}` as MessageKey
      const known = t(key)
      return known === key ? t('task.service', { name: row.name || '—' }) : known
    }
    if (row.kind === 'extension') return t('task.extension', { name: row.name })
    return t(`task.${row.kind}` as MessageKey)
  }

  return (
    <Modal label={t('cmd.task-manager')} width={680} onClose={closePanel}>
      <div className="modal-head">
        <h2>{t('cmd.task-manager')}</h2>
        <IconButton icon={X} label={t('common.close')} onClick={closePanel} />
      </div>
      <div className="modal-body">
        <div className="task-table" role="grid" aria-label={t('cmd.task-manager')}>
          <div className="task-row task-head label" role="row">
            <span role="columnheader">{t('task.name')}</span>
            <span role="columnheader">{t('task.memory')}</span>
            <span role="columnheader">{t('task.cpu')}</span>
            <span role="columnheader">{t('task.pid')}</span>
          </div>
          <div className="task-body">
            {rows.map((row) => (
              <button
                key={row.pid}
                type="button"
                role="row"
                aria-selected={row.pid === selected}
                className={`task-row ${row.pid === selected ? 'is-selected' : ''}`}
                onClick={() => setSelected(row.pid === selected ? null : row.pid)}
                onDoubleClick={() => row.tabs[0] && (api.activateTab(row.tabs[0].id), closePanel())}
              >
                <span className="task-name" role="gridcell">
                  {row.kind === 'tab' ? <Favicon src={row.tabs[0].favicon} url={row.tabs[0].url} size={16} /> : <Icon icon={ICONS[row.kind]} size={16} className="muted" />}
                  <span className="task-title">{name(row)}</span>
                  {row.tabs.length > 1 && <span className="muted task-more">{t('task.more', { n: row.tabs.length - 1 })}</span>}
                </span>
                <span className="mono" role="gridcell">
                  {formatBytes(settings.language, row.memory)}
                </span>
                <span className="mono" role="gridcell">
                  {row.cpu.toFixed(1)}
                </span>
                <span className="mono muted" role="gridcell">
                  {row.pid}
                </span>
              </button>
            ))}
          </div>
        </div>
        <div className="panel-actions task-actions">
          <span className="muted task-total">{t('task.total', { memory: formatBytes(settings.language, total) })}</span>
          <Button variant="quiet" disabled={chosen?.kind !== 'tab'} onClick={() => chosen && (api.endTask(chosen.pid), setSelected(null))}>
            {t('task.end')}
          </Button>
        </div>
      </div>
    </Modal>
  )
}
