import type { Decision, EffectType, Entry, Severity } from '../api/types'
import type { Config, EffectClass, FlagPolicy, ListKey, ManifestCfg } from './store'
import { ENFORCEMENT } from './store'

export const EFFECTS: EffectType[] = [
  'BALANCE_DECREASE', 'BALANCE_INCREASE', 'DELEGATE_GRANT', 'AUTHORITY_CHANGE', 'OWNER_CHANGE',
  'ACCOUNT_CREATE', 'ACCOUNT_CLOSE', 'ACCOUNT_FREEZE', 'PROGRAM_INVOKE',
]

export function classOf(m: ManifestCfg, t: EffectType): EffectClass {
  if (m.mustNot.includes(t)) return 'mustNot'
  if (m.must.some((p) => p.type === t)) return 'must'
  if (m.may.some((p) => p.type === t)) return 'may'
  return 'none'
}

/**
 * The engine's decision ladder (src/types/verdict.ts), first rule that holds:
 * any critical → deny; highs ≥ highsForDeny → deny; any high → flag;
 * mediums ≥ mediumsForFlag → flag; otherwise allow.
 */
export function decide(c: Record<Severity, number>, t: Config['thresholds']): Decision {
  if (c.critical > 0) return 'deny'
  if (c.high >= t.highsForDeny) return 'deny'
  if (c.high > 0) return 'flag'
  if (c.medium >= t.mediumsForFlag) return 'flag'
  return 'allow'
}

export const gateOutcome = (d: Decision, onFlag: FlagPolicy) =>
  d === 'allow' ? 'signed' : d === 'deny' ? 'refused' : onFlag === 'sign' ? 'signed' : onFlag === 'hold' ? 'held for a human' : 'refused'

export interface Change {
  area: 'Policy' | 'Gate' | 'Judge' | 'Deadlines' | 'Network' | 'Manifests' | 'Lists' | 'Agents'
  label: string
  from: string
  to: string
  live: boolean
}

const LIST_LABEL: Record<ListKey, string> = {
  allowlist: 'Allowlist', denylist: 'Deny-list', trustedSources: 'Trusted sources',
  knownCounterparties: 'Known counterparties', ownAccounts: 'Own accounts',
}

export function diff(a: Config, b: Config): Change[] {
  const out: Change[] = []
  const live = (k: string) => (ENFORCEMENT.live as readonly string[]).includes(k)
  const push = (area: Change['area'], label: string, from: unknown, to: unknown, key = '') => {
    if (JSON.stringify(from) !== JSON.stringify(to)) out.push({ area, label, from: String(from), to: String(to), live: live(key) })
  }
  push('Policy', 'Highs for deny', a.thresholds.highsForDeny, b.thresholds.highsForDeny)
  push('Policy', 'Mediums for flag', a.thresholds.mediumsForFlag, b.thresholds.mediumsForFlag)
  push('Policy', 'Fee ceiling (lamports)', a.thresholds.maxFeeLamports, b.thresholds.maxFeeLamports)
  push('Gate', 'On flag', a.gate.onFlag, b.gate.onFlag)
  push('Gate', 'Authorization lifetime (s)', a.gate.authorizationTtlSec, b.gate.authorizationTtlSec)
  push('Judge', 'LLM judge', a.judge.enabled ? 'on' : 'off', b.judge.enabled ? 'on' : 'off')
  push('Deadlines', 'Account read (ms)', a.deadlines.readMs, b.deadlines.readMs)
  push('Deadlines', 'Simulation (ms)', a.deadlines.simulateMs, b.deadlines.simulateMs)
  push('Deadlines', 'Total (ms)', a.deadlines.totalMs, b.deadlines.totalMs)
  push('Network', 'Network', a.network, b.network, 'network')

  for (const mb of b.manifests) {
    const ma = a.manifests.find((m) => m.tool === mb.tool)
    if (!ma) { out.push({ area: 'Manifests', label: mb.tool, from: '—', to: 'added', live: false }); continue }
    push('Manifests', `${mb.tool} · enabled`, ma.enabled, mb.enabled)
    push('Manifests', `${mb.tool} · account scope`, ma.accountScope, mb.accountScope)
    for (const t of EFFECTS) push('Manifests', `${mb.tool} · ${t}`, classOf(ma, t), classOf(mb, t))
  }

  for (const k of Object.keys(LIST_LABEL) as ListKey[]) {
    const A = new Set(a.lists[k].map((e) => e.value)), B = new Set(b.lists[k].map((e) => e.value))
    for (const v of B) if (!A.has(v)) out.push({ area: 'Lists', label: `${LIST_LABEL[k]} + ${v.slice(0, 6)}…${v.slice(-4)}`, from: '—', to: 'added', live: live(`lists.${k}`) })
    for (const v of A) if (!B.has(v)) out.push({ area: 'Lists', label: `${LIST_LABEL[k]} − ${v.slice(0, 6)}…${v.slice(-4)}`, from: 'listed', to: 'removed', live: live(`lists.${k}`) })
  }

  for (const gb of b.agents) {
    const ga = a.agents.find((x) => x.id === gb.id)
    if (!ga) { out.push({ area: 'Agents', label: gb.id, from: '—', to: 'registered', live: false }); continue }
    push('Agents', `${gb.id} · status`, ga.status, gb.status)
    push('Agents', `${gb.id} · on flag`, ga.onFlag, gb.onFlag)
    push('Agents', `${gb.id} · tools`, ga.tools.join(', '), gb.tools.join(', '))
    push('Agents', `${gb.id} · daily limit SOL`, ga.dailyLimitSol, gb.dailyLimitSol)
    push('Agents', `${gb.id} · network`, ga.network, gb.network)
  }
  for (const ga of a.agents) if (!b.agents.some((x) => x.id === ga.id)) out.push({ area: 'Agents', label: ga.id, from: 'registered', to: 'removed', live: false })
  return out
}

export interface Impact {
  considered: number
  flips: { entry: Entry; from: Decision; to: Decision }[]
  gate: { entry: Entry; from: string; to: string }[]
  overFee: Entry[]
  manifest: { entry: Entry; effect: EffectType; from: EffectClass; to: EffectClass }[]
}

/** Replays the session's real verdicts under the draft, using the engine's own rules. */
export function impact(entries: Entry[], from: Config, to: Config): Impact {
  const out: Impact = { considered: entries.length, flips: [], gate: [], overFee: [], manifest: [] }
  for (const e of entries) {
    const c = e.trace.decision.counts
    const a = decide(c, from.thresholds), b = decide(c, to.thresholds)
    if (a !== b) out.flips.push({ entry: e, from: a, to: b })
    const ga = gateOutcome(a, from.gate.onFlag), gb = gateOutcome(b, to.gate.onFlag)
    if (ga !== gb) out.gate.push({ entry: e, from: ga, to: gb })
    if (e.trace.transaction.fee && BigInt(e.trace.transaction.fee) > BigInt(to.thresholds.maxFeeLamports || '0')) out.overFee.push(e)
    const tool = e.trace.toolCall?.name
    const ma = from.manifests.find((m) => m.tool === tool), mb = to.manifests.find((m) => m.tool === tool)
    if (ma && mb) {
      for (const ef of new Set(e.trace.effects.map((x) => x.type))) {
        const ca = classOf(ma, ef), cb = classOf(mb, ef)
        if (ca !== cb) out.manifest.push({ entry: e, effect: ef, from: ca, to: cb })
      }
    }
  }
  return out
}
