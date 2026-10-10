import type { Effect } from './types'

export const short = (a: string | null | undefined, n = 4) =>
  !a ? '—' : a.length <= n * 2 + 1 ? a : `${a.slice(0, n)}…${a.slice(-n)}`

/* Programs the console can name; anything else shows as a shortened address. */
const PROGRAMS: Record<string, string> = {
  '11111111111111111111111111111111': 'System',
  TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA: 'Token',
  TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb: 'Token-2022',
  ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL: 'Associated Token',
  ComputeBudget111111111111111111111111111111: 'Compute Budget',
  So11111111111111111111111111111111111111112: 'wSOL',
}
export const named = (a: string | null | undefined) => (a && PROGRAMS[a]) || short(a)

export function lamports(v: string | null | undefined) {
  if (v == null) return '—'
  const n = Number(v) / 1e9
  return `${n.toLocaleString('en-US', { maximumFractionDigits: 9 })} SOL`
}

/** Amount as the engine reports it: lamports for SOL, raw base units otherwise. */
export function amount(e: Effect) {
  if (e.amount == null) return null
  if (e.mint === 'SOL') return lamports(e.amount)
  if (e.amount === '18446744073709551615') return 'u64::MAX (unlimited)'
  return `${Number(e.amount).toLocaleString('en-US')} units ${named(e.mint)}`
}

export const ago = (t: number) => {
  const s = Math.max(1, Math.round((Date.now() - t) / 1000))
  if (s < 60) return `${s}s ago`
  if (s < 3600) return `${Math.round(s / 60)}m ago`
  if (s < 86400) return `${Math.round(s / 3600)}h ago`
  return `${Math.round(s / 86400)}d ago`
}

export const EFFECT_GLYPH: Record<Effect['type'], string> = {
  BALANCE_DECREASE: '−',
  BALANCE_INCREASE: '+',
  DELEGATE_GRANT: '⇢',
  AUTHORITY_CHANGE: '⚿',
  OWNER_CHANGE: '⇄',
  ACCOUNT_CREATE: '▣',
  ACCOUNT_CLOSE: '✕',
  ACCOUNT_FREEZE: '❄',
  PROGRAM_INVOKE: '›',
}

export const EFFECT_LABEL: Record<Effect['type'], string> = {
  BALANCE_DECREASE: 'Balance out',
  BALANCE_INCREASE: 'Balance in',
  DELEGATE_GRANT: 'Delegate granted',
  AUTHORITY_CHANGE: 'Authority change',
  OWNER_CHANGE: 'Owner change',
  ACCOUNT_CREATE: 'Account created',
  ACCOUNT_CLOSE: 'Account closed',
  ACCOUNT_FREEZE: 'Account frozen',
  PROGRAM_INVOKE: 'Program call',
}
