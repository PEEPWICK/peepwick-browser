// The only bridge between a renderer and the main process, loaded into the browser
// UI view alone. Web pages never get this preload.
import { contextBridge, ipcRenderer, type IpcRendererEvent } from 'electron'
import { injectBrowserAction } from 'electron-chrome-extensions/browser-action'
import { IPC_EVENT, IPC_INVOKE, IPC_SEND, type PreloadBridge } from '@shared/ipc'

const bridge: PreloadBridge = {
  invoke: (method, ...args) => ipcRenderer.invoke(IPC_INVOKE, method, ...args),
  send: (method, ...args) => ipcRenderer.send(IPC_SEND, method, ...args),
  on: (event, listener) => {
    const handler = (_e: IpcRendererEvent, name: string, payload: unknown) => {
      if (name === event) listener(payload)
    }
    ipcRenderer.on(IPC_EVENT, handler)
    return () => ipcRenderer.removeListener(IPC_EVENT, handler)
  },
  platform: process.platform,
}

contextBridge.exposeInMainWorld('peepwick', bridge)

// Extension buttons (<browser-action-list>) in the toolbar.
injectBrowserAction()
