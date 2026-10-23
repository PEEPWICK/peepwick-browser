// The command bar ("Bak", Ctrl+K): open tabs, history and commands in one list.
import { useEffect, useRef, useState } from 'react'
import { CornerDownLeft, History, Search, SquareArrowOutUpRight, Star, Terminal } from 'lucide-react'
import type { MessageKey } from '@shared/i18n'
import type { Suggestion, SuggestionKind } from '@shared/types'
import { api, call } from '../api'
import { useUI } from '../context'
import { useT } from '../store'
import { Favicon, Icon, useOverlay } from '../ui'

const GROUP_LABEL: Partial<Record<SuggestionKind, MessageKey>> = {
  command: 'palette.commands',
  tab: 'palette.tabs',
  history: 'palette.history',
  bookmark: 'palette.bookmarks',
}

const ICONS = { command: Terminal, tab: SquareArrowOutUpRight, history: History, url: Search, search: Search, bookmark: Star }

export function Palette() {
  useOverlay(true)
  const t = useT()
  const { closePanel } = useUI()
  const input = useRef<HTMLInputElement>(null)
  const [text, setText] = useState('')
  const [items, setItems] = useState<Suggestion[]>([])
  const [selected, setSelected] = useState(0)
  const request = useRef(0)

  useEffect(() => {
    input.current?.focus()
  }, [])

  useEffect(() => {
    const id = ++request.current
    void call.palette(text).then((result) => {
      if (id !== request.current) return
      setItems(result)
      setSelected(0)
    })
  }, [text])

  const run = (item: Suggestion | undefined, newTab = false) => {
    if (!item) return
    closePanel()
    if (item.kind === 'command' && item.command) api.command(item.command)
    else if (item.kind === 'tab' && item.tabId !== undefined) api.activateTab(item.tabId)
    else api.openURL(item.value, newTab)
  }

  return (
    <div className="scrim" onMouseDown={closePanel}>
      <div className="palette" role="dialog" aria-label={t('palette.placeholder')} onMouseDown={(event) => event.stopPropagation()}>
        <div className="palette-input">
          <Icon icon={Search} size={18} />
          <input
            ref={input}
            value={text}
            placeholder={t('palette.placeholder')}
            spellCheck={false}
            aria-label={t('palette.placeholder')}
            onChange={(event) => setText(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'ArrowDown') {
                event.preventDefault()
                setSelected((s) => Math.min(items.length - 1, s + 1))
              } else if (event.key === 'ArrowUp') {
                event.preventDefault()
                setSelected((s) => Math.max(0, s - 1))
              } else if (event.key === 'Enter') {
                event.preventDefault()
                run(items[selected], event.altKey)
              } else if (event.key === 'Escape') {
                event.preventDefault()
                closePanel()
              }
            }}
          />
          <span className="label">Ctrl K</span>
        </div>
        <ul className="palette-list" role="listbox">
          {items.map((item, index) => {
            const header = index === 0 || items[index - 1].kind !== item.kind ? GROUP_LABEL[item.kind] : undefined
            return (
              <li key={`${item.kind}-${item.value}-${index}`} role="presentation">
                {header && <div className="palette-group label">{t(header)}</div>}
                <div
                  role="option"
                  aria-selected={index === selected}
                  className={`palette-item ${index === selected ? 'is-selected' : ''}`}
                  onMouseEnter={() => setSelected(index)}
                  onClick={() => run(item)}
                >
                  <span className="palette-icon">
                    {item.kind === 'tab' || item.kind === 'history' || item.kind === 'bookmark' ? (
                      <Favicon src={item.favicon} url={item.value} size={16} />
                    ) : (
                      <Icon icon={ICONS[item.kind]} size={16} />
                    )}
                  </span>
                  <span className="palette-title">{item.title}</span>
                  {index === selected && <Icon icon={CornerDownLeft} size={14} className="muted" />}
                </div>
              </li>
            )
          })}
        </ul>
      </div>
    </div>
  )
}
