import { EventEmitter } from 'node:events'

/**
 * App-wide notifications between main-process modules, so that modules such as
 * the shield or downloads can report changes without importing the window code.
 */
export const bus = new EventEmitter<{
  /** Focus, update or download state changed. */
  global: []
  settings: []
  bookmarks: []
  /** A tab's state changed; windows coalesce these into one UI update. */
  'window-dirty': [windowId: number]
  /** A host's certificate was checked for the first time this session. */
  security: []
}>()
bus.setMaxListeners(50)
