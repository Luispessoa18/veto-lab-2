import { useState } from 'react'
import { AnimatePresence, motion } from 'motion/react'
import { useEntries, store } from '../api/store'
import type { Decision } from '../api/types'
import { ago, short } from '../api/format'
import { DecisionBadge, SevCounts } from '../ui/Decision'
import Scramble from '../../components/Scramble'
import { go } from '../router'
import FeedChart, { type Range } from '../ui/FeedChart'
import { agentOf } from '../api/issues'

const FILTERS: (Decision | 'all')[] = ['all', 'deny', 'flag', 'allow']

export default function Actions() {
  const entries = useEntries()
  const [f, setF] = useState<Decision | 'all'>('all')
  const [range, setRange] = useState<Range>(null)
  const inRange = range ? entries.filter((e) => e.at >= range.from && e.at <= range.to) : entries
  const rows = inRange.filter((e) => f === 'all' || e.trace.decision.outcome === f)

  return (
    <div className="page">
      <header className="page__head page__head--row">
        <div>
          <Scramble as="p" text="actions · newest first" className="kicker" />
          <h1>Action log</h1>
        </div>
        <div className="seg" role="tablist" aria-label="Filter by decision">
          {FILTERS.map((x) => (
            <button key={x} role="tab" aria-selected={f === x} className={f === x ? 'is-on' : ''} onClick={() => setF(x)}>
              {f === x && <motion.span layoutId="seg-hl" className="seg__hl" transition={{ type: 'spring', stiffness: 500, damping: 38 }} />}
              <span>{x}</span>
              <em>{x === 'all' ? inRange.length : inRange.filter((e) => e.trace.decision.outcome === x).length}</em>
            </button>
          ))}
        </div>
      </header>

      <FeedChart entries={entries} range={range} onRange={setRange} />

      <div className="table" role="table" aria-label="Evaluated actions">
        <div className="table__head" role="row">
          <span>verdict</span><span>agent · tool</span><span>digest</span><span>simulation</span><span>findings</span><span>source</span><span>when</span>
        </div>
        <AnimatePresence initial={false}>
          {rows.map((e, i) => (
            <motion.button
              type="button"
              role="row"
              key={e.id}
              layout
              className="table__row"
              initial={{ opacity: 0, x: -10 }}
              animate={{ opacity: 1, x: 0, transition: { delay: Math.min(i, 12) * 0.025 } }}
              exit={{ opacity: 0, transition: { duration: 0.12 } }}
              onClick={() => go(`actions/${e.id}`)}
            >
              <span><DecisionBadge d={e.trace.decision.outcome} /></span>
              <span className="strong">{e.agent ? <><span className="mono-dim">{agentOf(e)} · </span>{e.trace.toolCall?.name ?? e.request.fixture}</> : e.trace.toolCall?.name ?? e.request.fixture}</span>
              <span className="mono-dim">{short(e.trace.transaction.messageDigest, 6)}</span>
              <span className={`sim sim--${e.trace.transaction.simulation}`}>{e.trace.transaction.simulation}</span>
              <span><SevCounts counts={e.trace.decision.counts} /></span>
              <span className={`src src--${e.source}`}>{e.source}</span>
              <span className="mono-dim">{ago(e.at)}</span>
            </motion.button>
          ))}
        </AnimatePresence>
        {!rows.length && <p className="empty">{range ? 'Nothing in that window.' : `No ${f} verdicts yet.`}</p>}
      </div>
      <button type="button" className="link link--muted" onClick={() => store.clear()}>reset session log</button>
    </div>
  )
}
