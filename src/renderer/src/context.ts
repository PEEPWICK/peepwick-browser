import { createContext, useContext } from 'react'
import type { ConfirmModel, MenuModel } from '@shared/types'

export type Panel =
  | { kind: 'shield'; anchor: DOMRect }
  | { kind: 'focus'; anchor: DOMRect }
  | { kind: 'downloads'; anchor: DOMRect }
  | { kind: 'workspace'; id: string | null; anchor: DOMRect }
  | { kind: 'palette' }
  | { kind: 'star'; anchor: DOMRect; created: boolean }
  | { kind: 'bookmark-edit'; id?: string; parentId?: string; folder?: boolean; rename?: boolean }
  | { kind: 'import' }
  | { kind: 'menu'; menu: MenuModel; context?: MenuModel }
  | { kind: 'confirm'; dialog: ConfirmModel }
  | { kind: 'site'; anchor: DOMRect }
  | { kind: 'clear-data' }
  | { kind: 'backup-export' }
  | { kind: 'backup-restore'; token: string; name: string }
  | { kind: 'certificate'; host: string }

export interface UIContextValue {
  panel: Panel | null
  openPanel: (panel: Panel) => void
  closePanel: () => void
  togglePanel: (panel: Panel) => void
  focusOmnibox: (text?: string) => void
  registerOmnibox: (focus: (text?: string) => void) => void
  findOpen: boolean
  setFindOpen: (open: boolean) => void
  showToast: (toast: Toast) => void
  /** Asks a yes/no question in the browser's own dialog. */
  ask: (question: Question) => Promise<boolean>
}

export type Question = Omit<ConfirmModel, 'id'>

export interface Toast {
  text: string
  action?: { label: string; run: () => void }
}

export const UIContext = createContext<UIContextValue>(null as unknown as UIContextValue)

export function useUI(): UIContextValue {
  return useContext(UIContext)
}
