import { useEffect, useState } from 'react'
import { AnimatePresence, motion } from 'motion/react'
import { config, useConfig, type ListKey } from '../config/store'
import { Enf, PageHead } from '../config/ui'
import { review } from '../api/api'
import Copy from '../ui/Copy'
import { ago, short } from '../api/format'

/* Provenance origins and the severity the engine gives a counterparty that
   comes from each (src/evaluation/provenance.ts). */
const LISTS: { k: ListKey; title: string; what: string; sev: string; kind: 'address' | 'source'; live?: boolean }[] = [
  { k: 'allowlist', title: 'Allowlist', what: 'Addresses the customer approved in advance. Receiving funds from the agent raises nothing.', sev: 'no finding', kind: 'address', live: true },
  { k: 'denylist', title: 'Deny-list', what: 'Addresses that must never receive anything. A match is denylist.address.', sev: 'critical', kind: 'address' },
  { k: 'trustedSources', title: 'Trusted sources', what: 'Documents, sites or tools whose text may introduce an address.', sev: 'low', kind: 'source' },
  { k: 'knownCounterparties', title: 'Known counterparties', what: 'Addresses seen before and recognised, but not pre-approved.', sev: 'medium', kind: 'address' },
  { k: 'ownAccounts', title: 'Own accounts', what: 'Wallets that belong to the operator. Moving funds between them is not a counterparty.', sev: 'no finding', kind: 'address' },
]
const BASE58 = /^[1-9A-HJ-NP-Za-km-z]{32,44}$/

export default function Lists() {
  const s = useConfig()
  const [tab, setTab] = useState<ListKey>('allowlist')
  const [val, setVal] = useState('')
  const [note, setNote] = useState('')
  const [engine, setEngine] = useState<{ allow: string[]; deny: string[] } | null>(null)
  useEffect(() => { review().then((r) => setEngine({ allow: r.review.live.allowlist, deny: r.review.live.denylist })).catch(() => {}) }, [])

  const L = LISTS.find((x) => x.k === tab)!
  const items = s.draft.lists[tab]
  const pub = new Set(s.history[0].config.lists[tab].map((e) => e.value))
  const v = val.trim()
  const error = !v ? '' : L.kind === 'address' && !BASE58.test(v) ? 'Not a Solana address: base58, 32 to 44 characters.' : items.some((e) => e.value === v) ? 'Already on this list.' : ''
  const add = () => {
    if (!v || error) return
    config.edit((c) => { c.lists[tab].unshift({ value: v, note: note.trim(), addedAt: Date.now() }) })
    setVal(''); setNote('')
  }
  const fromEngine = tab === 'allowlist' ? engine?.allow : tab === 'denylist' ? engine?.deny : undefined

  return (
    <div className="page">
      <PageHead kicker="configure · provenance lists" title="Who an agent may pay"
        lede="Every address a transaction reaches is traced back to where it came from. These lists decide how much that origin is trusted." />

      <div className="tabs" role="tablist" aria-label="Lists">
        {LISTS.map((x) => (
          <button key={x.k} role="tab" aria-selected={tab === x.k} className={tab === x.k ? 'is-on' : ''} onClick={() => setTab(x.k)}>
            {tab === x.k && <motion.span layoutId="tab-ul" className="tabs__ul" transition={{ type: 'spring', stiffness: 520, damping: 40 }} />}
            {x.title}<em>{s.draft.lists[x.k].length}</em>
          </button>
        ))}
      </div>

      <div className="cfggrid">
        <section className="panel">
          <div className="panel__head">
            <h2>{L.title}</h2>
            <span className="inline"><span className="mono-dim small">origin severity: <b className="ink">{L.sev}</b></span><Enf live={L.live} /></span>
          </div>
          <p className="mono-dim small listwhat">{L.what}</p>
          <form className="addrow" onSubmit={(e) => { e.preventDefault(); add() }}>
            <input id="list-value" className={`search ${error ? 'is-bad' : ''}`} placeholder={L.kind === 'address' ? 'Solana address' : 'Source, e.g. docs.company.com/invoices'} value={val} onChange={(e) => setVal(e.target.value)} aria-invalid={!!error} aria-describedby="list-err" />
            <input id="list-note" className="search" placeholder="Note (who, why)" value={note} onChange={(e) => setNote(e.target.value)} />
            <button type="submit" className="act act--primary" disabled={!v || !!error}>Add</button>
          </form>
          <p id="list-err" className="err" role="alert">{error}</p>
          <ul className="entries">
            <AnimatePresence initial={false}>
              {items.map((e) => (
                <motion.li key={e.value} layout initial={{ opacity: 0, y: -6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, x: 30 }}>
                  <Copy value={e.value} display={L.kind === 'address' ? short(e.value, 8) : e.value} />
                  <span className="mono-dim small">{e.note || 'no note'}</span>
                  <span className="mono-dim small">{pub.has(e.value) ? `published · ${ago(e.addedAt)}` : <em className="draftchip">draft</em>}</span>
                  <button type="button" className="act act--ghost" aria-label={`Remove ${e.value}`} onClick={() => config.edit((c) => { c.lists[tab] = c.lists[tab].filter((x) => x.value !== e.value) })}>remove</button>
                </motion.li>
              ))}
            </AnimatePresence>
            {!items.length && <li className="none">Empty. Add the first entry above.</li>}
          </ul>
        </section>

        <aside className="stack">
          <section className="panel">
            <div className="panel__head"><h2>Approved in review</h2><span className="mono-dim small">held by the engine</span></div>
            {fromEngine === undefined ? <p className="none">The review queue only writes the allowlist and deny-list.</p> :
              fromEngine.length ? (
                <ul className="entries entries--plain">
                  {fromEngine.map((a) => (
                    <li key={a}>
                      <Copy value={a} display={short(a, 8)} />
                      {!items.some((x) => x.value === a) && (
                        <button type="button" className="act act--ghost" onClick={() => config.edit((c) => { c.lists[tab].unshift({ value: a, note: 'from review queue', addedAt: Date.now() }) })}>copy to policy</button>
                      )}
                    </li>
                  ))}
                </ul>
              ) : <p className="none">Nothing approved yet.</p>}
          </section>
          <section className="panel">
            <div className="panel__head"><h2>How origins grade</h2></div>
            <ul className="grades">
              {[['own account', 'none'], ['user goal', 'none'], ['allowlist', 'none'], ['trusted source', 'low'], ['known counterparty', 'medium'], ['untrusted source', 'critical'], ['unexplained', 'critical']].map(([o, sv]) => (
                <li key={o}><span>{o}</span><span className={`sev sev--${sv === 'none' ? 'low' : sv}`}><i aria-hidden="true" />{sv}</span></li>
              ))}
            </ul>
          </section>
        </aside>
      </div>
    </div>
  )
}
