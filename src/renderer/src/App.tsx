import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { on } from './api'
import { Content } from './components/Content'
import { DownloadsPanel, FocusPanel, ShieldPanel, WorkspacePanel } from './components/Panels'
import { Onboarding } from './components/Onboarding'
import { Palette } from './components/Palette'
import { Sidebar } from './components/Sidebar'
import { Toolbar } from './components/Toolbar'
import { FindBar, PermissionBar } from './components/Bars'
import { UIContext, type Panel, type UIContextValue } from './context'
import { activeTab, useSnapshot } from './store'

function useTheme(): void {
  const { settings, state } = useSnapshot()
  const [systemDark, setSystemDark] = useState(() => matchMedia('(prefers-color-scheme: dark)').matches)
  useEffect(() => {
    const media = matchMedia('(prefers-color-scheme: dark)')
    const listener = () => setSystemDark(media.matches)
    media.addEventListener('change', listener)
    return () => media.removeEventListener('change', listener)
  }, [])
  const dark = state.incognito || settings.theme === 'dark' || (settings.theme === 'system' && systemDark)
  useEffect(() => {
    document.documentElement.dataset.theme = dark ? 'dark' : 'light'
    document.documentElement.lang = settings.language
  }, [dark, settings.language])
}

export function App() {
  useTheme()
  const { state, settings } = useSnapshot()
  const [panel, setPanel] = useState<Panel | null>(null)
  const [findOpen, setFindOpen] = useState(false)
  const omnibox = useRef<(text?: string) => void>(() => {})
  const tab = activeTab(state)

  const value = useMemo<UIContextValue>(
    () => ({
      panel,
      openPanel: setPanel,
      closePanel: () => setPanel(null),
      togglePanel: (next) => setPanel((current) => (current?.kind === next.kind ? null : next)),
      focusOmnibox: (text) => omnibox.current(text),
      registerOmnibox: (focus) => {
        omnibox.current = focus
      },
      findOpen,
      setFindOpen,
    }),
    [panel, findOpen],
  )

  const anchorFor = useCallback((selector: string): DOMRect => {
    const el = document.querySelector(selector)
    return el ? el.getBoundingClientRect() : new DOMRect(16, 16, 0, 0)
  }, [])

  useEffect(
    () =>
      on('command', (command) => {
        switch (command.type) {
          case 'focus-omnibox':
            setPanel(null)
            omnibox.current(command.text)
            break
          case 'palette':
            setPanel((current) => (current?.kind === 'palette' ? null : { kind: 'palette' }))
            break
          case 'find':
            setPanel(null)
            setFindOpen(true)
            window.dispatchEvent(new Event('pw-find-focus'))
            break
          case 'downloads':
            setPanel((current) => (current?.kind === 'downloads' ? null : { kind: 'downloads', anchor: anchorFor('[data-anchor="downloads"]') }))
            break
          case 'shield':
            setPanel({ kind: 'shield', anchor: anchorFor('[data-anchor="shield"]') })
            break
          case 'focus-panel':
            setPanel({ kind: 'focus', anchor: anchorFor('[data-anchor="focus"]') })
            break
          case 'escape':
            setFindOpen(false)
            break
        }
      }),
    [anchorFor],
  )

  // The find bar belongs to the page it was opened on.
  useEffect(() => setFindOpen(false), [tab?.id])

  const classes = [
    'app',
    settings.sidebarCollapsed ? 'is-collapsed' : '',
    state.fullscreen ? 'is-fullscreen' : '',
    state.incognito ? 'is-incognito' : '',
  ].join(' ')

  return (
    <UIContext.Provider value={value}>
      <div className={classes}>
        <Sidebar />
        <main className="main">
          <Toolbar />
          {state.permission && <PermissionBar prompt={state.permission} />}
          {findOpen && tab && !tab.page && <FindBar tab={tab} onClose={() => setFindOpen(false)} />}
          <Content />
        </main>
      </div>
      {panel?.kind === 'shield' && <ShieldPanel anchor={panel.anchor} />}
      {panel?.kind === 'focus' && <FocusPanel anchor={panel.anchor} />}
      {panel?.kind === 'downloads' && <DownloadsPanel anchor={panel.anchor} />}
      {panel?.kind === 'workspace' && <WorkspacePanel id={panel.id} anchor={panel.anchor} />}
      {panel?.kind === 'palette' && <Palette />}
      {!settings.onboarded && !state.incognito && <Onboarding />}
    </UIContext.Provider>
  )
}
