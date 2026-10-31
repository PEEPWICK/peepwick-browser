/// <reference types="vite/client" />
declare global {
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
