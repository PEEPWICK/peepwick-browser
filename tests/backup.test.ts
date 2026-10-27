import { describe, expect, it } from 'vitest'
import { emptyTree, isDraftTree, links, mergeTrees, toDraft, type BookmarkTree, type DraftTree } from '@shared/bookmarks'
import { BackupError, decryptBackup, encryptBackup, looksLikeBackup } from '../src/main/backupfile'

// The real cost (2^17) takes half a second per call; the format is the same.
const FAST = { logN: 10, r: 8, p: 1 }

describe('backup file', () => {
  const payload = { format: 'peepwick-backup', version: 1, createdAt: 1, settings: { theme: 'dark' }, note: 'ğüşiöç' }

  it('round-trips with the right password', async () => {
    const file = await encryptBackup(payload, 'doğru parola', FAST)
    expect(looksLikeBackup(file)).toBe(true)
    await expect(decryptBackup(file, 'doğru parola')).resolves.toEqual(payload)
  })

  it('never stores the contents in the clear', async () => {
    const file = await encryptBackup(payload, 'doğru parola', FAST)
    expect(file.includes(Buffer.from('peepwick-backup'))).toBe(false)
    expect(file.includes(Buffer.from('dark'))).toBe(false)
  })

  it('rejects a wrong password', async () => {
    const file = await encryptBackup(payload, 'doğru parola', FAST)
    await expect(decryptBackup(file, 'yanlış parola')).rejects.toEqual(new BackupError('password'))
  })

  it('notices a changed byte anywhere, header included', async () => {
    const file = await encryptBackup(payload, 'doğru parola', FAST)
    for (const index of [10, 30, file.length - 20, file.length - 1]) {
      const copy = Buffer.from(file)
      copy[index] ^= 1
      await expect(decryptBackup(copy, 'doğru parola')).rejects.toBeInstanceOf(BackupError)
    }
  })

  it('refuses files that are not backups, and absurd key parameters', async () => {
    await expect(decryptBackup(Buffer.from('not a backup at all, just some text'), 'x')).rejects.toEqual(new BackupError('format'))
    const file = await encryptBackup(payload, 'p', FAST)
    file[5] = 40
    await expect(decryptBackup(file, 'p')).rejects.toEqual(new BackupError('format'))
  })

  it('salts every file, so the same data never encrypts the same way', async () => {
    const a = await encryptBackup(payload, 'p', FAST)
    const b = await encryptBackup(payload, 'p', FAST)
    expect(a.equals(b)).toBe(false)
  })
})

describe('restoring bookmarks', () => {
  let n = 0
  const id = () => `id${++n}`

  function sample(): BookmarkTree {
    const tree = emptyTree('', '')
    tree.bar.children.push(
      { type: 'url', id: 'a', title: 'GitHub', url: 'https://github.com/', addedAt: 1 },
      { type: 'folder', id: 'f', title: 'İş', addedAt: 1, children: [{ type: 'url', id: 'b', title: 'Electron', url: 'https://www.electronjs.org/', addedAt: 1 }] },
    )
    return tree
  }

  it('turns a tree into a draft without ids and back', () => {
    const draft = toDraft(sample())
    expect(isDraftTree(draft)).toBe(true)
    expect(JSON.stringify(draft)).not.toContain('"id"')
  })

  it('merges into folders with the same name and skips addresses already there', () => {
    const tree = sample()
    const incoming: DraftTree = {
      bar: [
        { type: 'url', title: 'GitHub again', url: 'https://github.com' },
        { type: 'folder', title: 'İş', children: [{ type: 'url', title: 'Electron', url: 'https://www.electronjs.org/' }, { type: 'url', title: 'Vite', url: 'https://vite.dev/' }] },
        { type: 'folder', title: 'Yeni', children: [{ type: 'url', title: 'MDN', url: 'https://developer.mozilla.org/' }] },
      ],
      other: [{ type: 'url', title: 'Unsafe', url: 'javascript:alert(1)' }],
    }
    expect(mergeTrees(tree, incoming, id)).toBe(2)
    expect(tree.bar.children.filter((c) => c.type === 'folder' && c.title === 'İş')).toHaveLength(1)
    expect(links(tree).map((l) => l.title)).toEqual(['GitHub', 'Electron', 'Vite', 'MDN'])
    expect(tree.other.children).toHaveLength(0)
  })

  it('adds nothing the second time', () => {
    const tree = sample()
    const draft = toDraft(sample())
    expect(mergeTrees(tree, draft, id)).toBe(0)
  })

  it('rejects malformed drafts', () => {
    expect(isDraftTree({ bar: [{ type: 'url', title: 1 }], other: [] })).toBe(false)
    expect(isDraftTree(null)).toBe(false)
  })
})
