import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { api, on } from './api'
import { Content } from './components/Content'
import { DownloadsPanel, FocusPanel, MediaPanel, ShieldPanel, WorkspacePanel } from './components/Panels'
import { Onboarding } from './components/Onboarding'
import { Palette } from './components/Palette'
import { Sidebar } from './components/Sidebar'
import { Toolbar } from './components/Toolbar'
import { FindBar, PermissionBar } from './components/Bars'
import { BookmarksBar } from './components/BookmarksBar'
import { EditDialog, ImportDialog, StarPanel, ToastBar } from './components/BookmarkDialogs'
import { BackupExportDialog, BackupRestoreDialog } from './components/Backup'
import { ClearDataDialog } from './components/ClearData'
import { ConfirmDialog } from './components/Confirm'
import { MenuPopup } from './components/Menu'
import { AddressDialog, PasswordDialog } from './components/Autofill'
import { CertificateDialog, SitePanel } from './components/SiteInfo'
import { SaveLoginBar } from './components/SaveLogin'
import { TaskManagerDialog } from './components/TaskManager'
import { ListenBar, canListen } from './components/ListenBar'
import { TranslateBar } from './components/TranslateBar'
import { Tooltips } from './components/Tooltips'
import { UIContext, type Panel, type Question, type Toast, type UIContextValue } from './context'
import type { SaveLoginPrompt } from '@shared/types'
import { activeTab, useSnapshot, useT } from './store'

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
  const [listening, setListening] = useState(false)
  const [toast, setToast] = useState<Toast | null>(null)
  const [saveLogin, setSaveLogin] = useState<SaveLoginPrompt | null>(null)
  const closeToast = useCallback(() => setToast(null), [])
  const t = useT()
  const tRef = useRef(t)
  tRef.current = t
  const omnibox = useRef<(text?: string) => void>(() => {})
  const tab = activeTab(state)
  const activeTabRef = useRef(tab)
  activeTabRef.current = tab
  // Questions the UI asks itself get negative ids; the main process's are positive.
  const questions = useRef(new Map<number, (ok: boolean) => void>())
  const nextQuestion = useRef(0)
  const ask = useCallback(
    (question: Question) =>
      new Promise<boolean>((resolve) => {
        const id = --nextQuestion.current
        questions.current.set(id, resolve)
        setPanel({ kind: 'confirm', dialog: { id, ...question } })
      }),
    [],
  )
  const answer = (id: number, ok: boolean) => {
    if (id > 0) return api.confirmReply(id, ok)
    questions.current.get(id)?.(ok)
    questions.current.delete(id)
  }

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
      listening,
      setListening,
      showToast: setToast,
      ask,
    }),
    [panel, findOpen, listening, ask],
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
          case 'focus-page-search':
            setPanel(null)
            if (activeTabRef.current?.page === 'newtab') window.dispatchEvent(new Event('pw-page-search'))
            else omnibox.current()
            break
          case 'bookmark-star':
            setPanel({ kind: 'star', anchor: anchorFor('[data-anchor="star"]'), created: command.created === true })
            break
          case 'bookmark-edit':
            setPanel({ kind: 'bookmark-edit', id: command.id, parentId: command.parentId, folder: command.folder })
            break
          case 'import-bookmarks':
            setPanel({ kind: 'import' })
            break
          case 'menu': {
            const menu = command.menu
            // A right-click menu from a folder menu opens on top of it.
            setPanel((current) =>
              menu.over !== undefined && current?.kind === 'menu' && current.menu.id === menu.over ? { ...current, context: menu } : { kind: 'menu', menu },
            )
            break
          }
          case 'save-login':
            setSaveLogin(command.prompt)
            break
          case 'passwords-changed':
            window.dispatchEvent(new Event('pw-passwords-changed'))
            break
          case 'clear-data':
            setPanel({ kind: 'clear-data' })
            break
          case 'task-manager':
            setPanel((current) => (current?.kind === 'task-manager' ? null : { kind: 'task-manager' }))
            break
          case 'history-search':
            window.dispatchEvent(new CustomEvent('pw-history-search', { detail: command.text }))
            break
          case 'history-changed':
            window.dispatchEvent(new Event('pw-history-changed'))
            break
          case 'backup-restore':
            setPanel({ kind: 'backup-restore', token: command.token, name: command.name })
            break
          case 'certificate':
            setPanel({ kind: 'certificate', host: command.host })
            break
          case 'bookmark-rename':
            setPanel({ kind: 'bookmark-edit', id: command.id, rename: true })
            break
          case 'confirm':
            setPanel({ kind: 'confirm', dialog: command.dialog })
            break
          case 'reading-changed':
            window.dispatchEvent(new Event('pw-reading-changed'))
            break
          case 'toast':
            setToast({ text: command.text })
            break
          case 'bookmarks-removed':
            setToast({ text: tRef.current('bookmarks.removed', { n: command.count }), action: { label: tRef.current('bookmarks.undo'), run: () => api.bookmarkUndo() } })
            break
        }
      }),
    [anchorFor],
  )

  // The find bar and read aloud belong to the page they were opened on.
  useEffect(() => {
    setFindOpen(false)
    setListening(false)
  }, [tab?.id])

  const showBar =
    !state.incognito && !state.fullscreen && (settings.bookmarksBar === 'always' || (settings.bookmarksBar === 'newtab' && tab?.page === 'newtab'))

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
          {/* Everything above the pages is painted here, so no gap shows the window behind. */}
          <div className="chrome-top">
            <Toolbar />
            {showBar && <BookmarksBar />}
            {saveLogin && <SaveLoginBar prompt={saveLogin} onDone={() => setSaveLogin(null)} />}
            {toast && <ToastBar toast={toast} onClose={closeToast} />}
            {state.permission && <PermissionBar prompt={state.permission} />}
            {findOpen && tab && !tab.page && <FindBar tab={tab} onClose={() => setFindOpen(false)} />}
            {listening && tab && canListen(tab) && <ListenBar key={tab.id} tab={tab} uiLang={settings.language} onClose={() => setListening(false)} />}
            {tab?.translation && <TranslateBar tab={tab} />}
          </div>
          <Content />
        </main>
      </div>
      {panel?.kind === 'shield' && <ShieldPanel anchor={panel.anchor} />}
      {panel?.kind === 'focus' && <FocusPanel anchor={panel.anchor} />}
      {panel?.kind === 'downloads' && <DownloadsPanel anchor={panel.anchor} />}
      {panel?.kind === 'media' && <MediaPanel anchor={panel.anchor} />}
      {panel?.kind === 'workspace' && <WorkspacePanel id={panel.id} anchor={panel.anchor} />}
      {panel?.kind === 'palette' && <Palette />}
      {panel?.kind === 'star' && <StarPanel anchor={panel.anchor} created={panel.created} />}
      {panel?.kind === 'bookmark-edit' && <EditDialog id={panel.id} parentId={panel.parentId} folder={panel.folder} rename={panel.rename} />}
      {panel?.kind === 'import' && <ImportDialog focus={panel.focus} />}
      {panel?.kind === 'site' && <SitePanel anchor={panel.anchor} />}
      {panel?.kind === 'clear-data' && <ClearDataDialog />}
      {panel?.kind === 'task-manager' && <TaskManagerDialog />}
      {panel?.kind === 'password-edit' && <PasswordDialog key={panel.id ?? 'new'} id={panel.id} />}
      {panel?.kind === 'address-edit' && <AddressDialog key={panel.id ?? 'new'} id={panel.id} />}
      {panel?.kind === 'backup-export' && <BackupExportDialog />}
      {panel?.kind === 'backup-restore' && <BackupRestoreDialog key={panel.token} token={panel.token} name={panel.name} />}
      {panel?.kind === 'certificate' && <CertificateDialog host={panel.host} />}
      {panel?.kind === 'confirm' && <ConfirmDialog key={panel.dialog.id} dialog={panel.dialog} onAnswer={(ok) => answer(panel.dialog.id, ok)} />}
      {panel?.kind === 'menu' && <MenuPopup key={panel.menu.id} menu={panel.menu} inactive={!!panel.context} />}
      {panel?.kind === 'menu' && panel.context && (
        <MenuPopup
          key={panel.context.id}
          menu={panel.context}
          onDismiss={() => setPanel((current) => (current?.kind === 'menu' ? { kind: 'menu', menu: current.menu } : current))}
        />
      )}
      <Tooltips />
      {!settings.onboarded && !state.incognito && <Onboarding />}
    </UIContext.Provider>
  )
}
