// Bookmark dialogs: the star panel (Ctrl+D), the edit dialog, the import dialog and
// the toast bar that offers "Undo" after a deletion.
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { FileUp, X } from 'lucide-react'
import { folderList, isBookmarkableURL, isInside, locate, urlKey, type BookmarkLink, type BookmarkTree } from '@shared/bookmarks'
import type { ImportResult, ImportSource } from '@shared/types'
import { api, call } from '../api'
import { useUI, type Toast } from '../context'
import { activeTab, useBookmarks, useSnapshot, useT } from '../store'
import { Button, Icon, IconButton, useOverlay } from '../ui'
import { Popover } from './Panels'

/** Display names for the two roots follow the interface language. */
export function useFolderName(): (id: string, title: string) => string {
  const t = useT()
  return (id, title) => (id === 'bar' ? t('bookmarks.bar') : id === 'other' ? t('bookmarks.other') : title)
}

function FolderSelect({ value, onChange, exclude }: { value: string; onChange: (id: string) => void; exclude?: string }) {
  const tree = useBookmarks()
  const name = useFolderName()
  const t = useT()
  const folders = folderList(tree).filter(({ folder }) => !exclude || !isInside(tree, folder.id, exclude))
  return (
    <select className="select" value={value} aria-label={t('bookmarks.folder')} onChange={(event) => onChange(event.target.value)}>
      {folders.map(({ folder, depth }) => (
        <option key={folder.id} value={folder.id}>
          {`${' '.repeat(depth)}${name(folder.id, folder.title)}`}
        </option>
      ))}
    </select>
  )
}

export function findLink(tree: BookmarkTree, url: string): BookmarkLink | null {
  const key = urlKey(url)
  let match: BookmarkLink | null = null
  const visit = (nodes: BookmarkTree['bar']['children']) => {
    for (const node of nodes) {
      if (match) return
      if (node.type === 'url' && urlKey(node.url) === key) match = node
      else if (node.type === 'folder') visit(node.children)
    }
  }
  visit(tree.bar.children)
  visit(tree.other.children)
  return match
}

// ── Star panel ───────────────────────────────────────────────────────────────

export function StarPanel({ anchor, created }: { anchor: DOMRect; created: boolean }) {
  const tree = useBookmarks()
  const { state } = useSnapshot()
  const { closePanel } = useUI()
  const t = useT()
  const tab = activeTab(state)
  const link = tab ? findLink(tree, tab.url) : null
  const located = link ? locate(tree, link.id) : null
  const [title, setTitle] = useState(link?.title ?? '')
  const [folder, setFolder] = useState(located?.parent?.id ?? 'bar')
  const input = useRef<HTMLInputElement>(null)

  useEffect(() => {
    input.current?.focus()
    input.current?.select()
  }, [])

  if (!link) return null

  const save = () => {
    if (title.trim() && title !== link.title) api.bookmarkUpdate(link.id, { title })
    if (folder !== located?.parent?.id) api.bookmarkMove([link.id], folder, Number.MAX_SAFE_INTEGER)
    closePanel()
  }

  return (
    <Popover anchor={anchor} label={t('bookmarks.editTitle')} width={320}>
      <div className="panel-head">
        <span className="label">{created ? t('bookmarks.added') : t('bookmarks.editTitle')}</span>
      </div>
      <label className="field">
        <span className="label">{t('bookmarks.name')}</span>
        <input ref={input} className="text-input" value={title} onChange={(event) => setTitle(event.target.value)} onKeyDown={(event) => event.key === 'Enter' && save()} />
      </label>
      <label className="field">
        <span className="label">{t('bookmarks.folder')}</span>
        <FolderSelect value={folder} onChange={setFolder} />
      </label>
      <div className="panel-actions">
        <Button
          variant="quiet"
          onClick={() => {
            api.bookmarkRemove([link.id])
            closePanel()
          }}
        >
          {t('bookmarks.remove')}
        </Button>
        <span className="spacer" />
        <Button
          variant="quiet"
          onClick={() => {
            closePanel()
            api.openPage('bookmarks')
          }}
        >
          {t('bookmarks.manager')}
        </Button>
        <Button variant="primary" onClick={save}>
          {t('bookmarks.done')}
        </Button>
      </div>
    </Popover>
  )
}

// ── Modal ────────────────────────────────────────────────────────────────────

export function Modal({ label, width = 440, children, onClose }: { label: string; width?: number; children: ReactNode; onClose: () => void }) {
  useOverlay(true)
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => event.key === 'Escape' && onClose()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])
  return (
    <div className="scrim" onMouseDown={onClose}>
      <div className="modal" role="dialog" aria-label={label} style={{ width }} onMouseDown={(event) => event.stopPropagation()}>
        {children}
      </div>
    </div>
  )
}

// ── Edit dialog: edit a bookmark or folder, or create a new one ────────────────

/** Edits a bookmark or folder, creates one, or (rename) changes only the name. */
export function EditDialog({ id, parentId, folder, rename = false }: { id?: string; parentId?: string; folder?: boolean; rename?: boolean }) {
  const tree = useBookmarks()
  const { state } = useSnapshot()
  const { closePanel } = useUI()
  const t = useT()
  const existing = id ? locate(tree, id) : null
  const node = existing?.node
  const isFolder = node ? node.type === 'folder' : !!folder
  const tab = activeTab(state)
  const [title, setTitle] = useState(node?.title ?? (isFolder ? '' : tab && !tab.page ? tab.title : ''))
  const [url, setUrl] = useState(node?.type === 'url' ? node.url : !isFolder && tab && !tab.page ? tab.url : '')
  const [target, setTarget] = useState(existing?.parent?.id ?? parentId ?? 'bar')
  const [error, setError] = useState('')
  const first = useRef<HTMLInputElement>(null)

  useEffect(() => {
    first.current?.focus()
    first.current?.select()
  }, [])

  const heading = rename
    ? t('bookmarks.renameTitle')
    : node
      ? isFolder
        ? t('bookmarks.editFolder')
        : t('bookmarks.editTitle')
      : isFolder
        ? t('bookmarks.newFolder')
        : t('bookmarks.newBookmark')

  const save = async () => {
    if (rename && node) {
      if (title.trim()) api.bookmarkUpdate(node.id, { title: title.trim() })
      closePanel()
      return
    }
    const cleanURL = url.trim() && !/^[a-z][a-z0-9+.-]*:/i.test(url.trim()) ? `https://${url.trim()}` : url.trim()
    if (!isFolder && !isBookmarkableURL(cleanURL)) {
      setError(t('bookmarks.invalidUrl'))
      return
    }
    if (node) {
      api.bookmarkUpdate(node.id, { title, ...(isFolder ? {} : { url: cleanURL }) })
      if (existing?.parent && target !== existing.parent.id) api.bookmarkMove([node.id], target, Number.MAX_SAFE_INTEGER)
    } else {
      await call.bookmarkCreate({ parentId: target, title: title || (isFolder ? t('bookmarks.newFolder') : cleanURL), ...(isFolder ? {} : { url: cleanURL }) })
    }
    closePanel()
  }

  if (id && !node) return null
  const isRoot = !!existing && !existing.parent
  return (
    <Modal label={heading} onClose={closePanel}>
      <div className="modal-head">
        <h2>{heading}</h2>
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
          <span className="label">{t('bookmarks.name')}</span>
          <input ref={first} className="text-input" value={title} disabled={isRoot} onChange={(event) => setTitle(event.target.value)} />
        </label>
        {!isFolder && !rename && (
          <label className="field">
            <span className="label">{t('bookmarks.url')}</span>
            <input
              className="text-input mono"
              value={url}
              spellCheck={false}
              onChange={(event) => {
                setUrl(event.target.value)
                setError('')
              }}
            />
            {error && <span className="field-error">{error}</span>}
          </label>
        )}
        {!isRoot && !rename && (
          <label className="field">
            <span className="label">{t('bookmarks.folder')}</span>
            <FolderSelect value={target} onChange={setTarget} exclude={isFolder ? node?.id : undefined} />
          </label>
        )}
        <div className="panel-actions">
          <Button variant="quiet" onClick={closePanel}>
            {t('common.cancel')}
          </Button>
          <Button variant="primary" type="submit">
            {t('bookmarks.save')}
          </Button>
        </div>
      </form>
    </Modal>
  )
}

// ── Import dialog ────────────────────────────────────────────────────────────

export function ImportDialog() {
  const { closePanel } = useUI()
  const t = useT()
  const [sources, setSources] = useState<ImportSource[] | null>(null)
  const [choice, setChoice] = useState('html')
  const [skip, setSkip] = useState(true)
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<ImportResult | null>(null)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    void call.importSources().then((list) => {
      setSources(list)
      if (list[0]) setChoice(list[0].id)
    })
  }, [])

  const run = async () => {
    setBusy(true)
    setFailed(false)
    try {
      const outcome = await call.importBookmarks(choice, skip)
      if (outcome) setResult(outcome)
    } catch {
      setFailed(true)
    } finally {
      setBusy(false)
    }
  }

  const lines = useMemo(() => {
    if (!result) return []
    const out: string[] = []
    if (result.links + result.folders === 0) out.push(t('import.nothing'))
    else out.push(t('import.done', { n: result.links }))
    if (result.folders > 0) out.push(t('import.folders', { n: result.folders }))
    if (result.duplicates > 0) out.push(t('import.duplicates', { n: result.duplicates }))
    if (result.skipped > 0) out.push(t('import.skipped', { n: result.skipped }))
    if (result.favicons > 0) out.push(t('import.favicons', { n: result.favicons }))
    return out
  }, [result, t])

  return (
    <Modal label={t('import.title')} width={520} onClose={closePanel}>
      <div className="modal-head">
        <h2>{t('import.title')}</h2>
        <IconButton icon={X} label={t('common.close')} onClick={closePanel} />
      </div>
      {result ? (
        <div className="modal-body">
          <ul className="import-result">
            {lines.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
          <div className="panel-actions">
            <Button variant="quiet" onClick={closePanel}>
              {t('common.close')}
            </Button>
            <Button
              variant="primary"
              onClick={() => {
                closePanel()
                api.openPage('bookmarks')
              }}
            >
              {t('import.show')}
            </Button>
          </div>
        </div>
      ) : (
        <div className="modal-body">
          <p className="setting-hint">{sources === null ? t('import.searching') : sources.length > 0 ? t('import.lead') : t('import.none')}</p>
          <div className="import-sources" role="radiogroup" aria-label={t('import.title')}>
            {sources?.map((source) => (
              <label key={source.id} className={`import-source ${choice === source.id ? 'is-active' : ''}`}>
                <input type="radio" name="source" checked={choice === source.id} onChange={() => setChoice(source.id)} />
                <span className="import-name">
                  <strong>{source.browser}</strong>
                  <span className="muted">{source.profile}</span>
                </span>
                <span className="mono muted">{t('import.links', { n: source.links })}</span>
              </label>
            ))}
            <label className={`import-source ${choice === 'html' ? 'is-active' : ''}`}>
              <input type="radio" name="source" checked={choice === 'html'} onChange={() => setChoice('html')} />
              <span className="import-name">
                <strong>{t('import.html')}</strong>
                <span className="muted">{t('import.htmlHint')}</span>
              </span>
              <Icon icon={FileUp} size={16} className="muted" />
            </label>
          </div>
          <label className="check">
            <input type="checkbox" checked={skip} onChange={(event) => setSkip(event.target.checked)} />
            <span>{t('import.skipDuplicates')}</span>
          </label>
          {failed && <p className="field-error">{t('import.failed')}</p>}
          <div className="panel-actions">
            <Button variant="quiet" onClick={closePanel}>
              {t('common.cancel')}
            </Button>
            <Button variant="primary" disabled={busy || sources === null} onClick={() => void run()}>
              {busy ? t('import.running') : t('import.run')}
            </Button>
          </div>
        </div>
      )}
    </Modal>
  )
}

// ── Toast bar ────────────────────────────────────────────────────────────────

export function ToastBar({ toast, onClose }: { toast: Toast; onClose: () => void }) {
  const t = useT()
  useEffect(() => {
    const timer = setTimeout(onClose, 8000)
    return () => clearTimeout(timer)
  }, [toast, onClose])
  return (
    <div className="bar toast-bar" role="status">
      <span className="bar-dot" aria-hidden />
      <p className="bar-text">{toast.text}</p>
      {toast.action && (
        <Button
          variant="secondary"
          onClick={() => {
            toast.action?.run()
            onClose()
          }}
        >
          {toast.action.label}
        </Button>
      )}
      <IconButton icon={X} label={t('common.close')} onClick={onClose} />
    </div>
  )
}
