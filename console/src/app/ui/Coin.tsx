import type { ReactNode } from 'react'
import { SiSolana, SiTether } from 'react-icons/si'
import type { Trace } from '../api/types'

/*
 * Coin marks for the assets a transaction actually moves. Marks are the
 * official shapes (Simple Icons), flat and unmodified, on a plain disc; USDC
 * has no Simple Icons mark, so it gets a neutral "$" disc rather than an
 * imitation of its logo. Unknown mints show their first letters.
 */
type CoinInfo = { sym: string; name: string; mark: ReactNode; wrapped?: boolean }

const DOLLAR = <span className="coin__txt">$</span>
const COINS: Record<string, CoinInfo> = {
  SOL: { sym: 'SOL', name: 'Solana', mark: <SiSolana /> },
  So11111111111111111111111111111111111111112: { sym: 'wSOL', name: 'Wrapped SOL', mark: <SiSolana />, wrapped: true },
  EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v: { sym: 'USDC', name: 'USD Coin', mark: DOLLAR },
  Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB: { sym: 'USDT', name: 'Tether USD', mark: <SiTether /> },
}

export function coinInfo(mint: string): CoinInfo {
  return COINS[mint] ?? { sym: mint.slice(0, 4), name: `token ${mint}`, mark: <span className="coin__txt">{mint.slice(0, 2)}</span> }
}

/** Distinct assets a trace moves or grants, in order of appearance. */
export function coinsOf(t: Trace): string[] {
  const out: string[] = []
  for (const e of t.effects) {
    if (!e.mint || e.type === 'PROGRAM_INVOKE' || e.type === 'ACCOUNT_CREATE' || e.type === 'ACCOUNT_CLOSE') continue
    if (!out.includes(e.mint)) out.push(e.mint)
  }
  return out
}

export function Coin({ mint, size = 'sm' }: { mint: string; size?: 'sm' | 'md' }) {
  const c = coinInfo(mint)
  return (
    <span className={`coin coin--${size} ${c.wrapped ? 'is-wrapped' : ''}`} title={c.name} aria-label={c.name} role="img">
      {c.mark}
    </span>
  )
}

export function CoinStack({ mints }: { mints: string[] }) {
  if (!mints.length) return null
  return <span className="coinstack">{mints.map((m) => <Coin key={m} mint={m} />)}</span>
}
