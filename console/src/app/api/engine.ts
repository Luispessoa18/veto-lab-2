import { useEffect, useSyncExternalStore } from 'react'
import { ping } from './api'

/* Whether the Veto demo server answers through the proxy, re-checked every 10 s. */
type State = 'checking' | 'live' | 'offline'
let state: State = 'checking'
const subs = new Set<() => void>()
let timer = 0

async function check() {
  const next = (await ping()) ? 'live' : 'offline'
  if (next !== state) { state = next; subs.forEach((f) => f()) }
}

export function useEngine() {
  useEffect(() => {
    if (!timer) { check(); timer = window.setInterval(check, 10_000) }
  }, [])
  return useSyncExternalStore((f) => { subs.add(f); return () => subs.delete(f) }, () => state)
}
