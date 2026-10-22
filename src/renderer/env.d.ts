/// <reference types="vite/client" />
import type { PreloadBridge } from '@shared/ipc'

declare global {
  interface Window {
    peepwick: PreloadBridge
  }

  namespace React.JSX {
    interface IntrinsicElements {
      'browser-action-list': React.DetailedHTMLProps<React.HTMLAttributes<HTMLElement>, HTMLElement> & {
        partition?: string
        tab?: string
        alignment?: string
      }
    }
  }
}

export {}
