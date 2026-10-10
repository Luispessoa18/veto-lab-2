import type { ApprovalKind, EvalRequest, Review, Subject, Trace } from './types'
import { SNAPSHOT_REVIEW, SNAPSHOT_TRACES } from './fixtures'

/*
 * Talks to the Veto demo server through the /veto proxy (see vite.config.ts).
 * Every call falls back to recorded snapshots when the server is not running,
 * and says which one answered so the UI never passes a snapshot off as live.
 */
const BASE = '/veto'

export type Source = 'live' | 'snapshot'

async function call<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(BASE + path, {
    ...init,
    headers: { 'content-type': 'application/json' },
    signal: AbortSignal.timeout(8000),
  })
  if (!res.ok) throw new Error(`${res.status}`)
  const body = await res.json()
  if (body && typeof body === 'object' && 'error' in body) throw new EngineError(String(body.error))
  return body as T
}

/** The engine answered and refused the request; do not fall back to a snapshot. */
export class EngineError extends Error {}

export async function ping(): Promise<boolean> {
  try { await call('/review'); return true } catch { return false }
}

export async function evaluate(req: EvalRequest): Promise<{ trace: Trace; source: Source }> {
  try {
    return { trace: await call<Trace>('/evaluate', { method: 'POST', body: JSON.stringify(req) }), source: 'live' }
  } catch (e) {
    if (e instanceof EngineError) throw e
    const snap = SNAPSHOT_TRACES[req.fixture]
    if (!snap) throw new Error('engine offline: start the Veto demo server (see console/README.md)')
    return { trace: structuredClone(snap), source: 'snapshot' }
  }
}

/* Approvals made while offline live only in this tab, applied to the snapshot. */
let offline: Review = structuredClone(SNAPSHOT_REVIEW)

export async function review(): Promise<{ review: Review; source: Source }> {
  try { return { review: await call<Review>('/review'), source: 'live' } }
  catch (e) {
    if (e instanceof EngineError) throw e
    return { review: offline, source: 'snapshot' }
  }
}

export async function approve(
  kind: ApprovalKind,
  subject: Subject,
  shown: { considered: number; changed: number; speculative: boolean },
): Promise<{ review: Review; source: Source }> {
  try {
    const r = await call<Review>('/review/approve', {
      method: 'POST',
      body: JSON.stringify({ kind, subject, shown, by: 'console' }),
    })
    return { review: r, source: 'live' }
  } catch (e) {
    if (e instanceof EngineError) throw e
    offline = structuredClone(offline)
    if (kind === 'trust_address') offline.live.allowlist.push(subject.value)
    else if (kind === 'block_address') offline.live.denylist.push(subject.value)
    else offline.live.pending.push({ kind, subject })
    offline.items = offline.items.map((i) => (i.subject.value === subject.value ? { ...i, stance: kind } : i))
    return { review: offline, source: 'snapshot' }
  }
}
