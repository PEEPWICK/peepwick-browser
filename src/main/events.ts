import { EventEmitter } from 'node:events'

/**
 * App-wide notifications between main-process modules, so that modules such as
 * the shield or downloads can report changes without importing the window code.
 */
export const bus = new EventEmitter<{
  /** Focus, update or download state changed. */
  global: []
  settings: []
  /** A tab's state changed; windows coalesce these into one UI update. */
  'window-dirty': [windowId: number]
}>()
bus.setMaxListeners(50)
