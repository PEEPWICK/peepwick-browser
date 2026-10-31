// Typed client over the bridge to the Rust side.
import type { EventMap, EventName, InvokeMethods, SendMethods } from '@shared/ipc'
import { bridge } from './bridge'

type Invoke = { [K in keyof InvokeMethods]: (...args: Parameters<InvokeMethods[K]>) => Promise<ReturnType<InvokeMethods[K]>> }
type Send = { [K in keyof SendMethods]: (...args: Parameters<SendMethods[K]>) => void }


export const call = new Proxy({} as Invoke, {
  get: (_target, method: string) => (...args: unknown[]) => bridge.invoke(method, ...args),
})

export const api = new Proxy({} as Send, {
  get: (_target, method: string) => (...args: unknown[]) => bridge.send(method, ...args),
})

export function on<K extends EventName>(event: K, listener: (payload: EventMap[K]) => void): () => void {
  return bridge.on(event, listener as (payload: unknown) => void)
}
