// The bridge to the Rust side: the same three entry points the UI has always used
// (invoke, send, on), carried over Tauri commands and one event channel. Rust routes
// calls by method name, as the Electron main process did.
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { PreloadBridge } from '@shared/ipc'

/** Events from Rust arrive on one channel as { name, payload }. */
const EVENT = 'pw:event'

type Listener = (payload: unknown) => void
const listeners = new Map<string, Set<Listener>>()

void listen<{ name: string; payload: unknown }>(EVENT, ({ payload }) => {
  for (const listener of listeners.get(payload.name) ?? []) listener(payload.payload)
})

export const bridge: PreloadBridge = {
  invoke: (method, ...args) => invoke('pw_invoke', { method, args }),
  send: (method, ...args) => {
    void invoke('pw_send', { method, args }).catch((error: unknown) => console.error(`[bridge] ${method}`, error))
  },
  on: (event, listener) => {
    const set = listeners.get(event) ?? new Set<Listener>()
    set.add(listener)
    listeners.set(event, set)
    return () => set.delete(listener)
  },
  platform: 'linux',
}
