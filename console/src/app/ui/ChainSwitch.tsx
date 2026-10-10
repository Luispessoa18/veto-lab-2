import { useState } from 'react'
import { SiBitcoin, SiEthereum, SiPolygon, SiSolana, SiTether } from 'react-icons/si'
import { LuLock } from 'react-icons/lu'

/*
 * Which chain the gate investigates. Solana is the only one AVAL can simulate
 * and judge today; the rest are on the roadmap and shown locked, so the
 * direction is visible without pretending the infrastructure exists.
 */
type Chain = { k: string; label: string; mark: React.ReactNode; live: boolean; group: 'chains' | 'stablecoins' }

const DOLLAR = <span className="chain__txt">$</span>
const CHAINS: Chain[] = [
  { k: 'solana', label: 'Solana', mark: <SiSolana />, live: true, group: 'chains' },
  { k: 'ethereum', label: 'Ethereum', mark: <SiEthereum />, live: false, group: 'chains' },
  { k: 'bitcoin', label: 'Bitcoin', mark: <SiBitcoin />, live: false, group: 'chains' },
  { k: 'polygon', label: 'Polygon', mark: <SiPolygon />, live: false, group: 'chains' },
  { k: 'usdc', label: 'USDC', mark: DOLLAR, live: false, group: 'stablecoins' },
  { k: 'usdt', label: 'USDT', mark: <SiTether />, live: false, group: 'stablecoins' },
]

export default function ChainSwitch() {
  const [note, setNote] = useState<string | null>(null)
  return (
    <div className="chains">
      <span className="chains__label">investigating</span>
      <div className="chains__seg" role="radiogroup" aria-label="Chain under investigation">
        {CHAINS.map((c, i) => {
          const first = i === 0 || CHAINS[i - 1].group !== c.group
          return (
            <span key={c.k} className="chains__item">
              {first && c.group === 'stablecoins' && <span className="chains__sep" aria-hidden="true">stablecoins</span>}
              <button
                type="button"
                role="radio"
                aria-checked={c.live}
                aria-disabled={!c.live}
                className={`chain ${c.live ? 'is-on' : 'is-locked'}`}
                title={c.live ? `${c.label}: live` : `${c.label}: on the roadmap, not available yet`}
                onClick={() => setNote(c.live ? null : `${c.label} is on the roadmap. AVAL simulates and judges Solana transactions today.`)}
              >
                <span className="chain__mark">{c.mark}</span>
                <span>{c.label}</span>
                {c.live ? <i className="chain__dot" aria-hidden="true" /> : <LuLock className="chain__lock" aria-hidden="true" />}
              </button>
            </span>
          )
        })}
      </div>
      <span className="chains__note" role="status">{note}</span>
    </div>
  )
}
