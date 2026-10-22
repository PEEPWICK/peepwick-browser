import { useState, type DragEvent, type MouseEvent } from 'react'
import { Columns2, Download, PanelLeftClose, Plus, Settings, Timer, Volume2, VolumeX, X } from 'lucide-react'
import type { TabState, WorkspaceState } from '@shared/types'
import { api } from '../api'
import { DoubleEye } from '../brand'
import { useUI } from '../context'
import { activeTab, activeWorkspace, useSnapshot, useT } from '../store'
import { Favicon, Icon, IconButton, WORKSPACE_GLYPHS, remaining, useNow } from '../ui'

const DRAG_TYPE = 'application/x-peepwick-tab'

interface DragInfo {
  id: number
  workspaceId: string
}

let dragging: DragInfo | null = null

function TabIcon({ tab }: { tab: TabState }) {
  if (tab.loading) return <DoubleEye height={11} mode="scan" className="tab-loading" />
  if (tab.page) return <DoubleEye height={11} />
  return <Favicon src={tab.favicon} url={tab.url} size={16} />
}

function TabRow({
  tab,
  active,
  inSplit,
  workspace,
  onDropAt,
}: {
  tab: TabState
  active: boolean
  inSplit: boolean
  workspace: WorkspaceState
  onDropAt: (targetId: number, after: boolean, pinned: boolean) => void
}) {
  const t = useT()
  const [dropSide, setDropSide] = useState<'before' | 'after' | null>(null)

  const onMouseDown = (event: MouseEvent) => {
    if (event.button === 1) event.preventDefault()
  }
  const onAuxClick = (event: MouseEvent) => {
    if (event.button === 1) api.closeTab(tab.id)
  }
  const onDragStart = (event: DragEvent) => {
    dragging = { id: tab.id, workspaceId: workspace.id }
    event.dataTransfer.setData(DRAG_TYPE, String(tab.id))
    event.dataTransfer.effectAllowed = 'move'
  }
  const onDragOver = (event: DragEvent) => {
    if (!dragging || dragging.id === tab.id) return
    event.preventDefault()
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
    setDropSide(event.clientY > rect.top + rect.height / 2 ? 'after' : 'before')
  }
  const onDrop = (event: DragEvent) => {
    event.preventDefault()
    if (dropSide) onDropAt(tab.id, dropSide === 'after', false)
    setDropSide(null)
  }

  return (
    <div
      className={`tab-row ${active ? 'is-active' : ''} ${tab.sleeping ? 'is-sleeping' : ''} ${dropSide ? `drop-${dropSide}` : ''}`}
      draggable
      role="tab"
      aria-selected={active}
      title={tab.title}
      onMouseDown={onMouseDown}
      onClick={() => api.activateTab(tab.id)}
      onAuxClick={onAuxClick}
      onContextMenu={(event) => {
        event.preventDefault()
        api.tabMenu(tab.id, event.clientX, event.clientY)
      }}
      onDragStart={onDragStart}
      onDragEnd={() => (dragging = null)}
      onDragOver={onDragOver}
      onDragLeave={() => setDropSide(null)}
      onDrop={onDrop}
    >
      <span className="tab-icon">
        <TabIcon tab={tab} />
      </span>
      <span className="tab-title">{tab.title || t('tabs.untitled')}</span>
      {inSplit && <Icon icon={Columns2} size={14} className="tab-flag" />}
      {(tab.audible || tab.muted) && (
        <button
          type="button"
          className="tab-audio"
          aria-label={tab.muted ? t('tabs.unmute') : t('tabs.mute')}
          title={tab.muted ? t('tabs.unmute') : t('tabs.mute')}
          onClick={(event) => {
            event.stopPropagation()
            api.toggleMute(tab.id)
          }}
        >
          <Icon icon={tab.muted ? VolumeX : Volume2} size={14} />
        </button>
      )}
      <button
        type="button"
        className="tab-close"
        aria-label={t('tabs.close')}
        title={t('tabs.close')}
        onClick={(event) => {
          event.stopPropagation()
          api.closeTab(tab.id)
        }}
      >
        <Icon icon={X} size={14} />
      </button>
    </div>
  )
}

function PinnedTile({ tab, active, onDropAt }: { tab: TabState; active: boolean; onDropAt: (targetId: number, after: boolean, pinned: boolean) => void }) {
  return (
    <button
      type="button"
      className={`pinned-tile ${active ? 'is-active' : ''} ${tab.sleeping ? 'is-sleeping' : ''}`}
      title={tab.title}
      draggable
      onClick={() => api.activateTab(tab.id)}
      onAuxClick={(event) => event.button === 1 && api.closeTab(tab.id)}
      onContextMenu={(event) => {
        event.preventDefault()
        api.tabMenu(tab.id, event.clientX, event.clientY)
      }}
      onDragStart={(event) => {
        dragging = { id: tab.id, workspaceId: '' }
        event.dataTransfer.setData(DRAG_TYPE, String(tab.id))
      }}
      onDragEnd={() => (dragging = null)}
      onDragOver={(event) => dragging && dragging.id !== tab.id && event.preventDefault()}
      onDrop={(event) => {
        event.preventDefault()
        onDropAt(tab.id, false, true)
      }}
    >
      <TabIcon tab={tab} />
      {tab.audible && <span className="pinned-audio" />}
    </button>
  )
}

function WorkspaceSwitcher() {
  const { state } = useSnapshot()
  const t = useT()
  const { openPanel } = useUI()
  const [hover, setHover] = useState<string | null>(null)
  return (
    <div className="ws-switcher" role="tablist">
      {state.workspaces.map((workspace, index) => {
        const active = workspace.id === state.activeWorkspaceId
        return (
          <button
            key={workspace.id}
            type="button"
            role="tab"
            aria-selected={active}
            className={`ws-button ${active ? 'is-active' : ''} ${hover === workspace.id ? 'is-drop' : ''}`}
            title={`${workspace.name} · Alt+${index + 1}`}
            onClick={() => api.activateWorkspace(workspace.id)}
            onContextMenu={(event) => {
              event.preventDefault()
              openPanel({ kind: 'workspace', id: workspace.id, anchor: event.currentTarget.getBoundingClientRect() })
            }}
            onDragOver={(event) => {
              if (!dragging) return
              event.preventDefault()
              setHover(workspace.id)
            }}
            onDragLeave={() => setHover(null)}
            onDrop={(event) => {
              event.preventDefault()
              setHover(null)
              if (dragging) api.moveTab(dragging.id, workspace.id, workspace.tabIds.length)
            }}
          >
            <Icon icon={WORKSPACE_GLYPHS[workspace.icon]} size={16} />
          </button>
        )
      })}
      {state.workspaces.length < 12 && (
        <button
          type="button"
          className="ws-button ws-add"
          aria-label={t('ws.new')}
          title={t('ws.new')}
          onClick={(event) => openPanel({ kind: 'workspace', id: null, anchor: event.currentTarget.getBoundingClientRect() })}
        >
          <Icon icon={Plus} size={16} />
        </button>
      )}
    </div>
  )
}

function FocusButton({ collapsed }: { collapsed: boolean }) {
  const { global } = useSnapshot()
  const t = useT()
  const { togglePanel } = useUI()
  const now = useNow(1000, global.focus.active)
  const left = global.focus.endsAt - now
  return (
    <button
      type="button"
      data-anchor="focus"
      className={`tool-button focus-button ${global.focus.active ? 'is-running' : ''}`}
      title={t('focus.title')}
      onClick={(event) => togglePanel({ kind: 'focus', anchor: event.currentTarget.getBoundingClientRect() })}
    >
      <Icon icon={Timer} size={16} />
      {!collapsed && (
        <span className={global.focus.active ? 'mono' : ''}>{global.focus.active ? remaining(left) : t('focus.title')}</span>
      )}
      {global.focus.active && <span className="dot" />}
    </button>
  )
}

function DownloadsButton() {
  const { global } = useSnapshot()
  const t = useT()
  const { togglePanel } = useUI()
  const running = global.downloads.filter((d) => d.status === 'progressing')
  const total = running.reduce((sum, d) => sum + (d.total || 0), 0)
  const received = running.reduce((sum, d) => sum + d.received, 0)
  const progress = total > 0 ? received / total : 0
  return (
    <button
      type="button"
      data-anchor="downloads"
      className="tool-button icon-only"
      aria-label={t('downloads.title')}
      title={t('downloads.title')}
      onClick={(event) => togglePanel({ kind: 'downloads', anchor: event.currentTarget.getBoundingClientRect() })}
    >
      <Icon icon={Download} size={16} />
      {running.length > 0 && (
        <svg className="progress-ring" viewBox="0 0 24 24" aria-hidden>
          <circle cx="12" cy="12" r="10.5" pathLength="100" strokeDasharray={`${Math.max(4, progress * 100)} 100`} />
        </svg>
      )}
    </button>
  )
}

function UpdateCard() {
  const { global } = useSnapshot()
  const t = useT()
  if (global.update.status !== 'ready') return null
  return (
    <div className="update-card">
      <div>
        <div className="update-title">{t('update.ready')}</div>
        <div className="label">{global.update.version}</div>
      </div>
      <button type="button" className="button button-accent" onClick={() => api.updateInstall()}>
        {t('update.restart')}
      </button>
    </div>
  )
}

export function Sidebar() {
  const { state, settings, global, app } = useSnapshot()
  const t = useT()
  const { openPanel } = useUI()
  const workspace = activeWorkspace(state)
  const active = activeTab(state)
  const collapsed = settings.sidebarCollapsed
  const tabs = workspace.tabIds.map((id) => state.tabs[id]).filter((tab): tab is TabState => !!tab)
  const pinned = tabs.filter((tab) => tab.pinned)
  const regular = tabs.filter((tab) => !tab.pinned)
  const split = workspace.split
  const [listDrop, setListDrop] = useState(false)

  const dropAt = (targetId: number, after: boolean, pinnedDrop: boolean) => {
    if (!dragging) return
    const ids = workspace.tabIds
    let index = ids.indexOf(targetId) + (after ? 1 : 0)
    const from = ids.indexOf(dragging.id)
    if (from >= 0 && from < index) index--
    api.moveTab(dragging.id, workspace.id, index, pinnedDrop)
    dragging = null
  }

  return (
    <aside className="sidebar" aria-label={t('nav.sidebar')}>
      <header className="sidebar-head drag">
        <span className={`sidebar-mark ${state.incognito ? 'is-incognito' : ''}`} title={state.incognito ? t('incognito.title') : t('app.name')}>
          <DoubleEye height={collapsed ? 16 : 18} mode={state.incognito ? 'closed' : 'rest'} />
        </span>
        {state.incognito && !collapsed && <span className="label">{t('incognito.label')}</span>}
        {!state.incognito && !collapsed && app.stage !== 'stable' && (
          <span className="label stage-label" title={app.version}>
            {t(`stage.${app.stage}`)}
          </span>
        )}
        {!collapsed && (
          <IconButton
            icon={PanelLeftClose}
            label={t('cmd.toggle-sidebar')}
            className="no-drag sidebar-toggle"
            onClick={() => api.setSettings({ sidebarCollapsed: true })}
          />
        )}
      </header>

      {!collapsed && (
        <div className="ws-head">
          <button
            type="button"
            className="ws-name"
            title={t('ws.rename')}
            onClick={(event) => openPanel({ kind: 'workspace', id: workspace.id, anchor: event.currentTarget.getBoundingClientRect() })}
          >
            {workspace.name}
          </button>
          <span className="label">{t('tabs.count', { n: tabs.length })}</span>
        </div>
      )}

      {pinned.length > 0 && (
        <div
          className="pinned-grid"
          onDragOver={(event) => dragging && event.preventDefault()}
          onDrop={(event) => {
            event.preventDefault()
            if (dragging) api.moveTab(dragging.id, workspace.id, pinned.length, true)
          }}
        >
          {pinned.map((tab) => (
            <PinnedTile key={tab.id} tab={tab} active={tab.id === active?.id} onDropAt={dropAt} />
          ))}
        </div>
      )}

      <button type="button" className="new-tab-row" onClick={() => api.newTab()} title={`${t('tabs.new')} · Ctrl+T`}>
        <Icon icon={Plus} size={16} />
        {!collapsed && <span>{t('tabs.new')}</span>}
      </button>

      <div
        className={`tab-list ${listDrop ? 'is-drop' : ''}`}
        role="tablist"
        onDragOver={(event) => {
          if (!dragging) return
          event.preventDefault()
          if (event.target === event.currentTarget) setListDrop(true)
        }}
        onDragLeave={() => setListDrop(false)}
        onDrop={(event) => {
          setListDrop(false)
          if (!dragging || event.target !== event.currentTarget) return
          event.preventDefault()
          api.moveTab(dragging.id, workspace.id, workspace.tabIds.length, false)
          dragging = null
        }}
      >
        {regular.map((tab) => (
          <TabRow
            key={tab.id}
            tab={tab}
            active={tab.id === active?.id}
            inSplit={!!split && (split.left === tab.id || split.right === tab.id)}
            workspace={workspace}
            onDropAt={dropAt}
          />
        ))}
      </div>

      <footer className="sidebar-foot">
        {!collapsed && <UpdateCard />}
        <WorkspaceSwitcher />
        <div className="sidebar-tools">
          <FocusButton collapsed={collapsed} />
          <span className="spacer" />
          <DownloadsButton />
          <button
            type="button"
            className="tool-button icon-only"
            aria-label={t('menu.settings')}
            title={t('menu.settings')}
            onClick={() => api.openPage('settings')}
          >
            <Icon icon={Settings} size={16} />
            {global.update.status === 'ready' && collapsed && <span className="dot" />}
          </button>
        </div>
      </footer>
    </aside>
  )
}
