/* Shapes returned by the Veto demo server (src/demo/server.ts in the Veto repo).
   Field names follow the engine exactly; only what the console reads is typed. */

export type Decision = 'allow' | 'flag' | 'deny'
export type Severity = 'low' | 'medium' | 'high' | 'critical'
export type Provenance = 'retrieved' | 'memory' | 'tool_output'
export type EffectType =
  | 'BALANCE_DECREASE' | 'BALANCE_INCREASE' | 'DELEGATE_GRANT' | 'AUTHORITY_CHANGE'
  | 'OWNER_CHANGE' | 'ACCOUNT_CREATE' | 'ACCOUNT_CLOSE' | 'ACCOUNT_FREEZE' | 'PROGRAM_INVOKE'

export interface Effect {
  type: EffectType
  account: string
  owner: string | null
  mint: string | null
  amount: string | null
  counterparty: string | null
  program: string | null
  authorityKind: string | null
}

export interface Finding {
  check: string
  severity: Severity
  rule: string
  detail: Record<string, unknown>
  group: 'does' | 'compare' | 'read'
}

export interface Manifest {
  tool: string
  version: string
  must: EffectType[]
  may: EffectType[]
  mustNot: EffectType[]
  accountScope: string
}

export interface Trace {
  input: { goal: string; chunks: { text: string; provenance: string; source: string | null; trusted: boolean }[] }
  declarationSource: string
  toolCall: { name: string; args: Record<string, unknown>; manifest: Manifest | null } | null
  transaction: {
    messageDigest: string | null
    slot: number | null
    baselineSlot: number | null
    fee: string | null
    feePayer: string | null
    simulation: 'executed' | 'failed' | 'unavailable'
    simulationDetail: string | null
  }
  effects: Effect[]
  coverage: {
    referenced: number
    observed: number
    unobserved: number
    blindSpots: unknown[]
    tokenExtensions: unknown[]
    opaqueChanges: unknown[]
    inexactAmounts: unknown[]
    undecodable: unknown[]
    overSimulationLimit: unknown[]
  }
  counterparties: { address: string; origin: string; sources: string[]; severity: Severity | null }[]
  findings: Finding[]
  decision: {
    outcome: Decision
    counts: Record<Severity, number>
    thresholds: { highsForDeny: number; mediumsForFlag: number; maxFeeLamports: string }
    steps: { rule: string; held: boolean; detail: string }[]
  }
  policyVersion: string
  explanation: {
    verdict: string
    summary: string
    unattributed: boolean
    received: { text: string; origin: string }[]
    asked: string
    does: string[]
    reasons: string[]
    minor: string[]
  } | null
}

export type ApprovalKind = 'trust_address' | 'block_address' | 'write_manifest' | 'review_manifest' | 'allow_program'
export interface Subject { kind: 'address' | 'tool' | 'program'; value: string }

export interface Proposal {
  kind: ApprovalKind
  effect: string
  silences?: string[]
  retrospect: {
    considered: number
    changed: { actionId: string; at: number; from: Decision; to: Decision }[]
    speculative: boolean
  }
}

export interface ReviewItem {
  subject: Subject
  occurrences: number
  firstSeen: number
  lastSeen: number
  highestSeverity: Severity
  rules: { rule: string; severity: Severity; count: number }[]
  recentActions: string[]
  proposals: Proposal[]
  /** The approval a human already gave this subject, if any. */
  stance: ApprovalKind | null
}

export interface Review {
  records: number
  minOccurrences: number
  items: ReviewItem[]
  unattributable: { rule: string; severity: Severity; count: number }[]
  live: { allowlist: string[]; denylist: string[]; pending: { kind: ApprovalKind; subject: Subject }[] }
}

export interface EvalRequest {
  goal: string
  fixture: string
  provenance: Provenance
  /** Text the agent read before acting, and where it came from. */
  retrieved?: string
  source?: string
  /** What the agent said the amount was, to expose a declared/executed gap. */
  declaredAmount?: string
  /** Override the tool the fixture is attributed to. */
  tool?: string
  network?: 'mainnet' | 'devnet'
  /** From the published policy: addresses approved in advance. */
  allowlist?: string[]
}

/** One evaluation the console ran or loaded, with where its verdict came from. */
export interface Entry {
  id: string
  at: number
  request: EvalRequest
  trace: Trace
  source: 'live' | 'snapshot'
  ms: number
  /** Which agent proposed it (the Live gate's agents); absent for Scenario Lab runs. */
  agent?: string
}
