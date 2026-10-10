import { useSyncExternalStore } from 'react'
import type { Decision, Entry, Severity } from './types'

/*
 * Issues: repeated verdicts grouped the way Sentry groups errors. Every
 * flagged or denied action is fingerprinted by the rule that weighed most
 * (its highest-severity finding) and the agent that proposed it, so fourteen
 * identical denies read as one line with a count and a trend. What people do
 * with an issue (resolve, ignore, assign) is kept in this browser.
 */

export type IssueStatus = 'open' | 'resolved' | 'ignored' | 'regressed'
export const TEAM = ['lucas', 'eric', 'luis'] as const

export interface Issue {
  key: string
  rule: string
  agent: string
  severity: Severity
  decision: Decision
  entries: Entry[] // newest first
  first: number
  last: number
  status: IssueStatus
  owner: string | null
}

const SEV: Record<Severity, number> = { critical: 0, high: 1, medium: 2, low: 3 }

export const agentOf = (e: Entry) => e.agent ?? 'scenario lab'

/** The finding that weighed most on a verdict, or null when nothing rose above low. */
export function primary(e: Entry) {
  const f = [...e.trace.findings].sort((a, b) => SEV[a.severity] - SEV[b.severity])[0]
  return f && f.severity !== 'low' ? f : null
}

/* ── people's actions on issues, per browser ── */
type Mark = { status?: 'resolved' | 'ignored'; at?: number; owner?: string | null }
const KEY = 'aval.console.issues.v1'
let marks: Record<string, Mark> = {}
try { marks = JSON.parse(localStorage.getItem(KEY) || '{}') } catch { marks = {} }
const subs = new Set<() => void>()
function save() {
  try { localStorage.setItem(KEY, JSON.stringify(marks)) } catch { /* storage blocked */ }
  marks = { ...marks }
  subs.forEach((f) => f())
}
export const issueMarks = {
  resolve(key: string) { marks[key] = { ...marks[key], status: 'resolved', at: Date.now() }; save() },
  ignore(key: string) { marks[key] = { ...marks[key], status: 'ignored', at: Date.now() }; save() },
  reopen(key: string) { marks[key] = { ...marks[key], status: undefined, at: undefined }; save() },
  assign(key: string, owner: string | null) { marks[key] = { ...marks[key], owner }; save() },
}
export function useIssueMarks() {
  return useSyncExternalStore((f) => { subs.add(f); return () => subs.delete(f) }, () => marks)
}

export function buildIssues(entries: Entry[], m: Record<string, Mark>): Issue[] {
  const map = new Map<string, Issue>()
  for (const e of entries) {
    if (e.trace.decision.outcome === 'allow') continue
    const f = primary(e)
    const rule = f?.rule ?? 'decision.held'
    const agent = agentOf(e)
    const key = `${rule}::${agent}`
    let it = map.get(key)
    if (!it) {
      it = { key, rule, agent, severity: f?.severity ?? 'medium', decision: e.trace.decision.outcome, entries: [], first: e.at, last: e.at, status: 'open', owner: null }
      map.set(key, it)
    }
    it.entries.push(e)
    it.first = Math.min(it.first, e.at)
    it.last = Math.max(it.last, e.at)
    if (e.trace.decision.outcome === 'deny') it.decision = 'deny'
    if (f && SEV[f.severity] < SEV[it.severity]) it.severity = f.severity
  }
  for (const it of map.values()) {
    it.entries.sort((a, b) => b.at - a.at)
    const mk = m[it.key]
    it.owner = mk?.owner ?? null
    if (mk?.status === 'ignored') it.status = 'ignored'
    else if (mk?.status === 'resolved') it.status = it.last > (mk.at ?? 0) ? 'regressed' : 'resolved'
  }
  return [...map.values()].sort((a, b) => SEV[a.severity] - SEV[b.severity] || b.entries.length - a.entries.length || b.last - a.last)
}

/** Counts per time bucket across [from, to], for sparklines and the feed chart. */
export function buckets(entries: Entry[], from: number, to: number, n: number) {
  const span = Math.max(1, to - from)
  const out = Array.from({ length: n }, () => ({ allow: 0, flag: 0, deny: 0 }))
  for (const e of entries) {
    if (e.at < from || e.at > to) continue
    const i = Math.min(n - 1, Math.floor(((e.at - from) / span) * n))
    out[i][e.trace.decision.outcome]++
  }
  return out
}
