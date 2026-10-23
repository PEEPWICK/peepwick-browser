// Tooltips for the browser's own controls. Native tooltips take the system theme
// and font, so `title` attributes are taken over as they appear: the text moves to
// data-tip (and aria-description) and is drawn here.
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { api } from '../api'

const DELAY = 600
// After one tooltip, the next shows at once while the pointer moves along a row.
const WARM = 900
const GAP = 6
const MARGIN = 8

function adopt(el: Element): void {
  const title = el.getAttribute('title')
  if (title === null) return
  el.removeAttribute('title')
  if (!(el instanceof HTMLElement)) return
  el.dataset.tip = title
  if (title && !el.hasAttribute('aria-label')) el.setAttribute('aria-description', title)
}

/** Whether the rectangle covers a pane where a page's view sits above the UI. */
function overPage(rect: DOMRect): boolean {
  for (const pane of document.querySelectorAll('.pane:not(.is-drawn)')) {
    const p = pane.getBoundingClientRect()
    if (rect.left < p.right && rect.right > p.left && rect.top < p.bottom && rect.bottom > p.top) return true
  }
  return false
}

interface Tip {
  text: string
  target: DOMRect
}

function Bubble({ tip }: { tip: Tip }) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)

  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const { offsetWidth: width, offsetHeight: height } = el
    const { target } = tip
    const left = Math.max(MARGIN, Math.min(target.left + target.width / 2 - width / 2, window.innerWidth - MARGIN - width))
    let top = target.bottom + GAP
    if (top + height > window.innerHeight - MARGIN) top = target.top - GAP - height
    setPos({ left, top })
  }, [tip])

  // The pages sit above the UI: raise it while the tooltip reaches over one.
  useEffect(() => {
    if (!pos || !ref.current || !overPage(ref.current.getBoundingClientRect())) return
    api.setRaised(true)
    return () => api.setRaised(false)
  }, [pos])

  return (
    <div ref={ref} className="tooltip" role="tooltip" style={pos ?? { left: 0, top: 0, visibility: 'hidden' }}>
      {tip.text}
    </div>
  )
}

export function Tooltips() {
  const [tip, setTip] = useState<Tip | null>(null)

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined
    let current: HTMLElement | null = null
    let warmUntil = 0

    const observer = new MutationObserver((records) => {
      for (const record of records) {
        if (record.type === 'attributes') adopt(record.target as Element)
        else
          for (const node of record.addedNodes) {
            if (!(node instanceof Element)) continue
            adopt(node)
            node.querySelectorAll('[title]').forEach(adopt)
          }
      }
    })
    observer.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['title'] })
    document.body.querySelectorAll('[title]').forEach(adopt)

    const hide = () => {
      clearTimeout(timer)
      setTip((shown) => {
        if (shown) warmUntil = Date.now() + WARM
        return null
      })
    }

    const onOver = (event: PointerEvent) => {
      // composedPath reaches into shadow roots (the extension buttons).
      const el = event
        .composedPath()
        .find((node): node is HTMLElement => node instanceof HTMLElement && (node.hasAttribute('title') || 'tip' in node.dataset))
      if (el) adopt(el)
      if (el === current) return
      current = el ?? null
      hide()
      const text = el?.dataset.tip
      if (!el || !text) return
      timer = setTimeout(
        () => {
          if (current === el && el.isConnected) setTip({ text, target: el.getBoundingClientRect() })
        },
        Date.now() < warmUntil ? 0 : DELAY,
      )
    }

    const reset = () => {
      current = null
      hide()
    }

    document.addEventListener('pointerover', onOver, true)
    document.addEventListener('pointerdown', reset, true)
    document.addEventListener('wheel', reset, { capture: true, passive: true })
    document.addEventListener('keydown', reset, true)
    document.documentElement.addEventListener('pointerleave', reset)
    window.addEventListener('blur', reset)
    return () => {
      observer.disconnect()
      clearTimeout(timer)
      document.removeEventListener('pointerover', onOver, true)
      document.removeEventListener('pointerdown', reset, true)
      document.removeEventListener('wheel', reset, { capture: true })
      document.removeEventListener('keydown', reset, true)
      document.documentElement.removeEventListener('pointerleave', reset)
      window.removeEventListener('blur', reset)
    }
  }, [])

  return tip ? <Bubble tip={tip} /> : null
}
