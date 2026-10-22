import type { Tab } from './tab'

/** Lookup from a webContents id to the tab hosting it, for request and IPC routing. */
const byContents = new Map<number, Tab>()

export function registerContents(id: number, tab: Tab): void {
  byContents.set(id, tab)
}

export function unregisterContents(id: number): void {
  byContents.delete(id)
}

export function tabForContents(id: number | undefined): Tab | undefined {
  return id === undefined ? undefined : byContents.get(id)
}
