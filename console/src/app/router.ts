import { useSyncExternalStore } from 'react'

/* Hash routes: #/overview, #/evaluate, #/actions, #/actions/<id>, #/review */
const read = () => location.hash.replace(/^#\/?/, '').split('/').filter(Boolean)
let parts = read()
const subs = new Set<() => void>()
addEventListener('hashchange', () => { parts = read(); subs.forEach((f) => f()); scrollTo({ top: 0 }) })

export function useRoute() {
  return useSyncExternalStore((f) => { subs.add(f); return () => subs.delete(f) }, () => parts)
}
export const go = (path: string) => { location.hash = '/' + path }
