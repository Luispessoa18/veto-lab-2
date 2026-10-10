import { useSyncExternalStore } from 'react'
import type { EffectType } from '../api/types'

/*
 * Platform configuration: one draft the operator edits, and a history of
 * published versions. Field names follow the Veto engine (Thresholds,
 * ToolManifest, ProvenanceConfig, FlagPolicy) so a published version can be
 * handed to the engine's config API once it exists (backlog F3).
 *
 * Today the engine reads only some of this per request (`allowlist`,
 * `network`); `ENFORCEMENT` below says which, and the UI tags every setting.
 */

export type FlagPolicy = 'sign' | 'hold' | 'refuse'
export type Scope = 'args_only' | 'args_plus_derived' | 'open'
export type EffectClass = 'must' | 'may' | 'mustNot' | 'none'

export interface Pattern { type: EffectType; [k: string]: unknown }
export interface ManifestCfg {
  tool: string
  version: string
  args: Record<string, string>
  must: Pattern[]
  may: Pattern[]
  mustNot: EffectType[]
  accountScope: Scope
  enabled: boolean
}
export interface ListEntry { value: string; note: string; addedAt: number }
export type ListKey = 'allowlist' | 'denylist' | 'trustedSources' | 'knownCounterparties' | 'ownAccounts'
export interface Agent {
  id: string
  label: string
  wallet: string
  network: 'mainnet' | 'devnet'
  tools: string[]
  onFlag: FlagPolicy | 'inherit'
  status: 'active' | 'paused' | 'killed'
  dailyLimitSol: number
}
export interface Config {
  thresholds: { highsForDeny: number; mediumsForFlag: number; maxFeeLamports: string }
  gate: { onFlag: FlagPolicy; authorizationTtlSec: number }
  judge: { enabled: boolean; stricterOnly: true }
  deadlines: { readMs: number; simulateMs: number; totalMs: number }
  network: 'mainnet' | 'devnet'
  manifests: ManifestCfg[]
  lists: Record<ListKey, ListEntry[]>
  agents: Agent[]
}
export interface Published { id: string; at: number; by: string; note: string; config: Config }

/** What the running engine actually reads from the console today. */
export const ENFORCEMENT = {
  live: ['lists.allowlist', 'network'],
  note: 'Sent with every evaluation from the Scenario Lab. Everything else is staged policy until the engine exposes its config API.',
} as const

const DEMO_WALLET = '11111111111111111111111111111112'

export function engineDefaults(): Config {
  return {
    // The engine's severity ladder: 2 high → deny, 1 medium → flag, fee ceiling 0.01 SOL.
    thresholds: { highsForDeny: 2, mediumsForFlag: 1, maxFeeLamports: '10000000' },
    gate: { onFlag: 'sign', authorizationTtlSec: 60 },
    judge: { enabled: false, stricterOnly: true },
    // Normalizer deadlines from the engine's backlog item G2.
    deadlines: { readMs: 1500, simulateMs: 3000, totalMs: 5000 },
    network: 'mainnet',
    // Tool manifests come from the engine; none are bundled in this copy.
    manifests: [],
    lists: { allowlist: [], denylist: [], trustedSources: [], knownCounterparties: [], ownAccounts: [] },
    agents: [
      { id: 'treasury-07', label: 'Treasury rebalancer', wallet: DEMO_WALLET, network: 'mainnet', tools: ['swap', 'transfer_token'], onFlag: 'hold', status: 'active', dailyLimitSol: 250 },
      { id: 'payroll', label: 'Payroll runner', wallet: DEMO_WALLET, network: 'mainnet', tools: ['transfer_sol', 'transfer_token'], onFlag: 'inherit', status: 'active', dailyLimitSol: 40 },
      { id: 'yield-bot', label: 'Staking & yield', wallet: DEMO_WALLET, network: 'devnet', tools: ['stake_native', 'unstake_native', 'withdraw_stake'], onFlag: 'refuse', status: 'paused', dailyLimitSol: 100 },
    ],
  }
}

/* Short, stable id for a config: FNV-1a over its JSON. Same config, same id. */
export function versionId(c: Config) {
  const s = JSON.stringify(c)
  let h = 0x811c9dc5
  for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 0x01000193) }
  return 'p-' + (h >>> 0).toString(16).padStart(8, '0').slice(0, 7)
}

const KEY = 'aval.console.config.v1'
type State = { draft: Config; history: Published[] }

function initial(): State {
  try {
    const raw = localStorage.getItem(KEY)
    if (raw) return JSON.parse(raw)
  } catch { /* storage blocked */ }
  const base = engineDefaults()
  return { draft: structuredClone(base), history: [{ id: versionId(base), at: Date.now() - 86_400_000, by: 'engine defaults', note: 'Baseline: the engine’s default thresholds, no manifests bundled.', config: base }] }
}

let state: State = initial()
const subs = new Set<() => void>()
function commit(next: State) {
  state = next
  try { localStorage.setItem(KEY, JSON.stringify(state)) } catch { /* storage blocked */ }
  subs.forEach((f) => f())
}

export const config = {
  get: () => state,
  published: () => state.history[0].config,
  /** Apply a change to the draft with a recipe that mutates a copy. */
  edit(recipe: (d: Config) => void) {
    const d = structuredClone(state.draft)
    recipe(d)
    commit({ ...state, draft: d })
  },
  discard() { commit({ ...state, draft: structuredClone(state.history[0].config) }) },
  publish(note: string, by = 'console') {
    const cfg = structuredClone(state.draft)
    const v: Published = { id: versionId(cfg), at: Date.now(), by, note, config: cfg }
    commit({ ...state, history: [v, ...state.history].slice(0, 40) })
    return v
  },
  rollback(id: string) {
    const v = state.history.find((h) => h.id === id)
    if (v) commit({ ...state, draft: structuredClone(v.config) })
  },
  reset() { try { localStorage.removeItem(KEY) } catch { /* noop */ } commit(initial()) },
}

export function useConfig() {
  return useSyncExternalStore((f) => { subs.add(f); return () => subs.delete(f) }, () => state)
}
