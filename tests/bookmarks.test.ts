import { describe, expect, it } from 'vitest'
import {
  countNodes,
  decodeEntities,
  emptyTree,
  folderList,
  fromChromium,
  fromPlaces,
  isInside,
  locate,
  parseNetscape,
  toNetscape,
  urlKey,
  withoutDuplicates,
  type BookmarkTree,
  type DraftNode,
} from '@shared/bookmarks'

// A trimmed export from Chrome: the toolbar folder first, then other bookmarks at the top level.
const CHROME_EXPORT = `<!DOCTYPE NETSCAPE-Bookmark-file-1>
<!-- This is an automatically generated file. -->
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<DL><p>
    <DT><H3 ADD_DATE="1700000000" LAST_MODIFIED="0" PERSONAL_TOOLBAR_FOLDER="true">Yer işaretleri çubuğu</H3>
    <DL><p>
        <DT><A HREF="https://github.com/PEEPWICK" ADD_DATE="1700000001" ICON="data:image/png;base64,AAAA">PEEPWICK &amp; GitHub</A>
        <DT><H3 ADD_DATE="1700000002">İş</H3>
        <DL><p>
            <DT><A HREF="https://example.com/a?b=1&amp;c=2">Örnek</A>
            <DT><A HREF="javascript:alert(1)">Bookmarklet</A>
        </DL><p>
    </DL><p>
    <DT><A HREF="https://tr.wikipedia.org/">Vikipedi</A>
</DL><p>
`

// Firefox puts the toolbar inside the top-level list and uses place: smart folders.
const FIREFOX_EXPORT = `<!DOCTYPE NETSCAPE-Bookmark-file-1>
<meta http-equiv="Content-Type" content="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks Menu</H1>
<DL><p>
    <DT><A HREF="place:sort=8&maxResults=10">Most Visited</A>
    <DT><H3 ADD_DATE="1" LAST_MODIFIED="2" PERSONAL_TOOLBAR_FOLDER="true">Bookmarks Toolbar</H3>
    <DL><p>
        <DT><A HREF="https://www.mozilla.org/" ADD_DATE="3">Mozilla</A>
    </DL><p>
    <DT><H3>Other Bookmarks</H3>
    <DL><p>
        <DT><A HREF="https://news.ycombinator.com/">HN</A>
    </DL><p>
</DL>
`

describe('Netscape bookmark files', () => {
  it('reads a Chrome export into the bar and other bookmarks', () => {
    const { tree, skipped } = parseNetscape(CHROME_EXPORT)
    expect(skipped).toBe(1)
    expect(tree.bar).toHaveLength(2)
    expect(tree.bar[0]).toMatchObject({ type: 'url', title: 'PEEPWICK & GitHub', url: 'https://github.com/PEEPWICK', addedAt: 1_700_000_001_000 })
    const folder = tree.bar[1] as DraftNode & { type: 'folder' }
    expect(folder).toMatchObject({ type: 'folder', title: 'İş' })
    expect(folder.children).toEqual([{ type: 'url', title: 'Örnek', url: 'https://example.com/a?b=1&c=2', addedAt: undefined }])
    expect(tree.other).toEqual([{ type: 'url', title: 'Vikipedi', url: 'https://tr.wikipedia.org/', addedAt: undefined }])
  })

  it('reads a Firefox export and skips smart folders', () => {
    const { tree, skipped } = parseNetscape(FIREFOX_EXPORT)
    expect(skipped).toBe(1)
    expect(tree.bar.map((n) => n.title)).toEqual(['Mozilla'])
    expect(countNodes(tree.other)).toEqual({ links: 1, folders: 1 })
  })

  it('survives a round trip through our own export', () => {
    const tree: BookmarkTree = emptyTree('Yer imi çubuğu', 'Diğer yer imleri')
    tree.bar.children.push({ type: 'url', id: 'a', title: 'A "quoted" <title>', url: 'https://a.test/?x=1&y=2', addedAt: 1_700_000_000_000 })
    tree.bar.children.push({
      type: 'folder',
      id: 'f',
      title: 'Klasör',
      addedAt: 0,
      children: [{ type: 'url', id: 'b', title: 'B', url: 'https://b.test/', addedAt: 0 }],
    })
    tree.other.children.push({ type: 'url', id: 'c', title: 'C', url: 'https://c.test/', addedAt: 0 })
    const { tree: back } = parseNetscape(toNetscape(tree))
    expect(back.bar[0]).toMatchObject({ title: 'A "quoted" <title>', url: 'https://a.test/?x=1&y=2', addedAt: 1_700_000_000_000 })
    expect(countNodes(back.bar)).toEqual({ links: 2, folders: 1 })
    expect(back.other.map((n) => n.title)).toEqual(['C'])
  })

  it('decodes entities', () => {
    expect(decodeEntities('A &amp; B &#252;ber &#x131; &lt;tag&gt;')).toBe('A & B über ı <tag>')
  })
})

describe('Chromium and Firefox profiles', () => {
  it('converts Chromium JSON, with mobile bookmarks in their own folder', () => {
    const json = {
      roots: {
        bookmark_bar: {
          children: [{ type: 'url', name: 'GitHub', url: 'https://github.com/', date_added: '13340000000000000' }],
        },
        other: { children: [{ type: 'folder', name: 'Okunacaklar', children: [{ type: 'url', name: 'X', url: 'https://x.test/' }] }] },
        synced: { children: [{ type: 'url', name: 'Telefon', url: 'https://m.test/' }] },
      },
    }
    const { tree } = fromChromium(json, 'Mobil yer imleri')
    expect(tree.bar[0]).toMatchObject({ title: 'GitHub', url: 'https://github.com/' })
    // 13340000000000000 µs since 1601 is 2023-09-26.
    expect(new Date((tree.bar[0] as { addedAt: number }).addedAt).getUTCFullYear()).toBe(2023)
    expect(tree.other.map((n) => n.title)).toEqual(['Okunacaklar', 'Mobil yer imleri'])
  })

  it('rejects files that are not Chromium bookmarks', () => {
    expect(() => fromChromium({ hello: 1 }, 'Mobil')).toThrow()
  })

  it('builds a tree from Firefox places rows', () => {
    const rows = [
      { id: 1, type: 2, parent: 0, position: 0, title: '', url: null, dateAdded: null, guid: 'root________' },
      { id: 2, type: 2, parent: 1, position: 0, title: 'menu', url: null, dateAdded: null, guid: 'menu________' },
      { id: 3, type: 2, parent: 1, position: 1, title: 'toolbar', url: null, dateAdded: null, guid: 'toolbar_____' },
      { id: 5, type: 2, parent: 1, position: 3, title: 'unfiled', url: null, dateAdded: null, guid: 'unfiled_____' },
      { id: 10, type: 1, parent: 3, position: 1, title: 'İkinci', url: 'https://2.test/', dateAdded: 1_700_000_000_000_000, guid: 'a' },
      { id: 11, type: 1, parent: 3, position: 0, title: 'Birinci', url: 'https://1.test/', dateAdded: null, guid: 'b' },
      { id: 12, type: 1, parent: 2, position: 0, title: 'Menüde', url: 'https://menu.test/', dateAdded: null, guid: 'c' },
      { id: 13, type: 3, parent: 2, position: 1, title: null, url: null, dateAdded: null, guid: 'sep' },
      { id: 14, type: 1, parent: 5, position: 0, title: 'Smart', url: 'place:sort=8', dateAdded: null, guid: 'd' },
    ]
    const { tree, skipped } = fromPlaces(rows, 'Yer imleri menüsü', 'Mobil')
    expect(tree.bar.map((n) => n.title)).toEqual(['Birinci', 'İkinci'])
    expect((tree.bar[1] as { addedAt: number }).addedAt).toBe(1_700_000_000_000)
    expect(tree.other).toHaveLength(1)
    expect(tree.other[0]).toMatchObject({ type: 'folder', title: 'Yer imleri menüsü' })
    expect(skipped).toBe(1)
  })
})

describe('tree helpers', () => {
  const tree: BookmarkTree = emptyTree('Bar', 'Other')
  tree.bar.children.push({
    type: 'folder',
    id: 'work',
    title: 'İş',
    addedAt: 0,
    children: [{ type: 'folder', id: 'deep', title: 'Derin', addedAt: 0, children: [{ type: 'url', id: 'x', title: 'X', url: 'https://x.test/', addedAt: 0 }] }],
  })

  it('locates nodes with their path', () => {
    const found = locate(tree, 'x')
    expect(found?.parent?.id).toBe('deep')
    expect(found?.path.map((f) => f.id)).toEqual(['bar', 'work', 'deep'])
    expect(locate(tree, 'missing')).toBeNull()
  })

  it('knows when a folder would move into itself', () => {
    expect(isInside(tree, 'deep', 'work')).toBe(true)
    expect(isInside(tree, 'work', 'deep')).toBe(false)
  })

  it('lists folders with depth', () => {
    expect(folderList(tree).map((f) => `${f.depth}:${f.folder.id}`)).toEqual(['0:bar', '1:work', '2:deep', '0:other'])
  })

  it('drops duplicates on import', () => {
    const draft = {
      bar: [
        { type: 'url' as const, title: 'X again', url: 'https://x.test' },
        { type: 'folder' as const, title: 'Empty after filter', children: [{ type: 'url' as const, title: 'X', url: 'https://x.test/#top' }] },
        { type: 'url' as const, title: 'New', url: 'https://new.test/' },
      ],
      other: [{ type: 'url' as const, title: 'New twice', url: 'https://new.test' }],
    }
    const { tree: result, duplicates } = withoutDuplicates(draft, new Set([urlKey('https://x.test/')]))
    expect(result.bar.map((n) => n.title)).toEqual(['New'])
    expect(result.other).toEqual([])
    expect(duplicates).toBe(3)
  })
})
