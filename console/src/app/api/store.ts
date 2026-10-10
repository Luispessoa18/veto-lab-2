import { useSyncExternalStore } from 'react'
import type { Entry } from './types'

/*
 * The console's action log: every evaluation run from this browser. Kept in
 * localStorage so a reload keeps the session; storage can be unavailable
 * (private windows), in which case the log just lives in memory.
 */
const KEY = 'aval.console.entries.v1'
const MAX = 60

function seed(): Entry[] {
  // Starts empty: verdicts appear as the Live gate or the Scenario Lab runs them.
  return []
}

function load(): Entry[] {
  try {
    const raw = localStorage.getItem(KEY)
    if (raw) return JSON.parse(raw)
  } catch { /* storage blocked */ }
  return seed()
}

let entries: Entry[] = load()
const subs = new Set<() => void>()

function save() {
  try { localStorage.setItem(KEY, JSON.stringify(entries)) } catch { /* storage blocked */ }
  subs.forEach((f) => f())
}

export const store = {
  add(e: Entry) { entries = [e, ...entries].slice(0, MAX); save() },
  get(id: string) { return entries.find((e) => e.id === id) },
  clear() { entries = seed(); save() },
}

export function useEntries() {
  return useSyncExternalStore(
    (f) => { subs.add(f); return () => subs.delete(f) },
    () => entries,
  )
}
