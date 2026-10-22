// A small external store for state pushed from the main process.
import { useCallback, useSyncExternalStore } from 'react'
import type { InitialData } from '@shared/ipc'
import { translate, type MessageKey, type Vars } from '@shared/i18n'
import type { AppInfo, GlobalState, Settings, TabState, WindowState, WorkspaceState } from '@shared/types'
import { call, on } from './api'

export interface Snapshot {
  state: WindowState
  global: GlobalState
  settings: Settings
  app: AppInfo
}

let snapshot: Snapshot | null = null
const listeners = new Set<() => void>()

function emit(): void {
  for (const listener of listeners) listener()
}

export async function initStore(): Promise<void> {
  const data: InitialData = await call.ready()
  snapshot = data
  on('state', (state) => {
    if (snapshot) snapshot = { ...snapshot, state }
    emit()
  })
  on('global', (global) => {
    if (snapshot) snapshot = { ...snapshot, global }
    emit()
  })
  on('settings', (settings) => {
    if (snapshot) snapshot = { ...snapshot, settings }
    emit()
  })
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function useSnapshot(): Snapshot {
  return useSyncExternalStore(subscribe, () => snapshot as Snapshot)
}

export function useWindow(): WindowState {
  return useSnapshot().state
}

export function useSettings(): Settings {
  return useSnapshot().settings
}

export function useGlobal(): GlobalState {
  return useSnapshot().global
}

export function useT(): (key: MessageKey, vars?: Vars) => string {
  const lang = useSettings().language
  return useCallback((key: MessageKey, vars?: Vars) => translate(lang, key, vars), [lang])
}

export function activeWorkspace(state: WindowState): WorkspaceState {
  return state.workspaces.find((w) => w.id === state.activeWorkspaceId) ?? state.workspaces[0]
}

export function activeTab(state: WindowState): TabState | null {
  const id = activeWorkspace(state).activeTabId
  return id === null ? null : (state.tabs[id] ?? null)
}
