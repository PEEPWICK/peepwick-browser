import { createContext, useContext } from 'react'

export type Panel =
  | { kind: 'shield'; anchor: DOMRect }
  | { kind: 'focus'; anchor: DOMRect }
  | { kind: 'downloads'; anchor: DOMRect }
  | { kind: 'workspace'; id: string | null; anchor: DOMRect }
  | { kind: 'palette' }

export interface UIContextValue {
  panel: Panel | null
  openPanel: (panel: Panel) => void
  closePanel: () => void
  togglePanel: (panel: Panel) => void
  focusOmnibox: (text?: string) => void
  registerOmnibox: (focus: (text?: string) => void) => void
  findOpen: boolean
  setFindOpen: (open: boolean) => void
}

export const UIContext = createContext<UIContextValue>(null as unknown as UIContextValue)

export function useUI(): UIContextValue {
  return useContext(UIContext)
}
