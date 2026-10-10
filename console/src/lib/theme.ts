import { useSyncExternalStore } from 'react'

/*
 * One theme for the site and the console (same origin, same storage key).
 * The inline script in each HTML head applies it before first paint; this
 * module owns switching, following the OS until the person picks one, and the
 * colors canvases need (they cannot read CSS variables on their own).
 */
export type Theme = 'dark' | 'light'
const KEY = 'aval.theme'
const root = document.documentElement
const mq = matchMedia('(prefers-color-scheme: light)')

const stored = (): Theme | null => {
  try { const v = localStorage.getItem(KEY); return v === 'light' || v === 'dark' ? v : null } catch { return null }
}
export const current = (): Theme => (root.dataset.theme === 'light' ? 'light' : 'dark')

/* RGB triplets for canvas drawing. Kept here, not read from CSS: in dev the
   stylesheet is injected after this module runs, so a CSS read can come back
   empty. Values mirror --ink-rgb / --bg-rgb / --deny-rgb in styles.css. */
const PALETTES = {
  dark: { ink: '236,236,236', bg: '5,5,5', deny: '255,59,47' },
  light: { ink: '13,13,12', bg: '242,242,239', deny: '224,38,26' },
}
export const palette = { get ink() { return PALETTES[current()].ink }, get bg() { return PALETTES[current()].bg }, get deny() { return PALETTES[current()].deny } }

const subs = new Set<() => void>()
function apply(t: Theme) {
  root.dataset.theme = t
  document.querySelector('meta[name=theme-color]')?.setAttribute('content', t === 'light' ? '#f2f2ef' : '#050505')
  subs.forEach((f) => f())
}
mq.addEventListener('change', () => { if (!stored()) apply(mq.matches ? 'light' : 'dark') })
// Another tab (site vs console) switched: follow it.
addEventListener('storage', (e) => { if (e.key === KEY && (e.newValue === 'light' || e.newValue === 'dark')) apply(e.newValue) })

/** Switch theme; the new one grows in a circle from (x, y) where supported. */
export function toggleTheme(x = innerWidth - 40, y = 40) {
  const next: Theme = current() === 'dark' ? 'light' : 'dark'
  try { localStorage.setItem(KEY, next) } catch { /* storage blocked: still switch for this page */ }
  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches
  const doc = document as Document & { startViewTransition?: (cb: () => void) => { ready: Promise<void> } }
  if (!doc.startViewTransition || reduced) return apply(next)
  const r = Math.hypot(Math.max(x, innerWidth - x), Math.max(y, innerHeight - y))
  const vt = doc.startViewTransition(() => apply(next))
  vt.ready.then(() => {
    root.animate(
      { clipPath: [`circle(0px at ${x}px ${y}px)`, `circle(${r}px at ${x}px ${y}px)`] },
      { duration: 700, easing: 'cubic-bezier(.16,1,.3,1)', pseudoElement: '::view-transition-new(root)' },
    )
  }).catch(() => {})
}

export function useTheme() {
  return useSyncExternalStore((f) => { subs.add(f); return () => subs.delete(f) }, current)
}
