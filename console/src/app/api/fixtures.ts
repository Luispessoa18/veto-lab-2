import type { Review, Trace } from './types'

/* The six transactions the Veto demo server ships as fixtures. This public
   copy carries no recorded engine output: with the engine offline there are
   no snapshots to fall back to, and the console says the engine is offline. */
export const FIXTURES = [
  { name: 'transfer-sol', title: 'Transfer SOL', blurb: 'Native SOL leaves the wallet for another address.', art: '◎ ──▶ ◎' },
  { name: 'token-to-wallet', title: 'Token to wallet', blurb: 'Wrapped SOL lands in a freshly created account.', art: '▣ ──▶ ◎' },
  { name: 'unlimited-approve', title: 'Unlimited approve', blurb: 'A delegate gets authority over the full u64 amount.', art: '◎ ══∞═▶ ?' },
  { name: 'create-ata', title: 'Create token account', blurb: 'An associated token account is opened and funded.', art: '+ ▣' },
  { name: 'close-account', title: 'Close account', blurb: 'A token account is closed and its rent reclaimed.', art: '▣ ─✕' },
  { name: 'rent-failure', title: 'Rent failure', blurb: 'A transaction that would fail if sent right now.', art: '◎ ─/─ ✕' },
] as const

export const SNAPSHOT_TRACES: Record<string, Trace> = {}

export const SNAPSHOT_REVIEW: Review = { records: 0, minOccurrences: 3, items: [], unattributable: [], live: { allowlist: [], denylist: [], pending: [] } }
