import { useEffect, useState } from 'react'
import { motion } from 'motion/react'
import { useEntries } from '../api/store'
import { review as fetchReview } from '../api/api'
import type { Review, Severity } from '../api/types'
import { ago, short } from '../api/format'
import { DecisionBadge, SevCounts } from '../ui/Decision'
import Meter from '../ui/Meter'
import Counter from '../../components/Counter'
import Scramble from '../../components/Scramble'
import { go } from '../router'
import VetoInspector from '../ui/VetoInspector'

const stagger = { hidden: {}, show: { transition: { staggerChildren: 0.06 } } }
const rise = { hidden: { opacity: 0, y: 14 }, show: { opacity: 1, y: 0, transition: { type: 'spring' as const, stiffness: 300, damping: 30 } } }

export default function Overview() {
  const entries = useEntries()
  const [rev, setRev] = useState<Review | null>(null)
  useEffect(() => { fetchReview().then((r) => setRev(r.review)).catch(() => {}) }, [])

  const total = entries.length || 1
  const by = { allow: 0, flag: 0, deny: 0 }
  const rules = new Map<string, { n: number; sev: Severity }>()
  for (const e of entries) {
    by[e.trace.decision.outcome]++
    for (const f of e.trace.findings) {
      const r = rules.get(f.rule) ?? { n: 0, sev: f.severity }
      r.n++
      rules.set(f.rule, r)
    }
  }
  const top = [...rules.entries()].sort((a, b) => b[1].n - a[1].n).slice(0, 6)
  const maxRule = top[0]?.[1].n ?? 1
  const pending = rev?.items.filter((i) => !i.stance).length ?? 0

  return (
    <motion.div variants={stagger} initial="hidden" animate="show" className="page">
      <motion.header variants={rise} className="page__head">
        <Scramble as="p" text="overview · this console session" className="kicker" />
        <h1>Every action, judged.</h1>
      </motion.header>

      <motion.section variants={rise} className="kpis">
        <div className="kpi">
          <span className="kpi__label">evaluated</span>
          <Counter to={entries.length} className="kpi__n" />
          <span className="kpi__sub">{entries.filter((e) => e.source === 'live').length} live · {entries.filter((e) => e.source === 'snapshot').length} snapshot</span>
        </div>
        {(['deny', 'flag', 'allow'] as const).map((d) => (
          <div key={d} className={`kpi kpi--${d}`}>
            <span className="kpi__label">{d === 'deny' ? 'denied' : d === 'flag' ? 'flagged' : 'allowed'}</span>
            <Counter to={by[d]} className="kpi__n" />
            <Meter value={by[d] / total} cols={18} className={`meter--${d}`} />
          </div>
        ))}
      </motion.section>

      <motion.div variants={rise}>
        <VetoInspector entries={entries} />
      </motion.div>

      <div className="grid2">
        <motion.section variants={rise} className="panel">
          <div className="panel__head"><h2>Latest verdicts</h2><a href="#/actions" className="link">all actions →</a></div>
          <ul className="vlist">
            {entries.slice(0, 7).map((e) => (
              <li key={e.id}>
                <button type="button" className="vrow" onClick={() => go(`overview/inspect/${e.id}`)}>
                  <DecisionBadge d={e.trace.decision.outcome} />
                  <span className="vrow__tool">{e.trace.toolCall?.name ?? e.request.fixture}</span>
                  <span className="vrow__digest">{short(e.trace.transaction.messageDigest, 5)}</span>
                  <SevCounts counts={e.trace.decision.counts} />
                  <span className="vrow__at">{ago(e.at)}</span>
                </button>
              </li>
            ))}
          </ul>
        </motion.section>

        <div className="stack">
          <motion.section variants={rise} className="panel panel--accent" onClick={() => go('review')} role="link" tabIndex={0}
            onKeyDown={(e) => e.key === 'Enter' && go('review')}>
            <div className="panel__head"><h2>Review queue</h2><span className="link">open →</span></div>
            <div className="bigline">
              <Counter to={pending} className="bigline__n" />
              <span>subjects waiting for a human<br />across {rev?.records ?? '—'} recorded actions</span>
            </div>
          </motion.section>

          <motion.section variants={rise} className="panel">
            <div className="panel__head"><h2>Rules that fired most</h2></div>
            <ul className="rules">
              {top.map(([rule, r]) => (
                <li key={rule}>
                  <code>{rule}</code>
                  <Meter value={r.n / maxRule} cols={12} className={`meter--sev-${r.sev}`} />
                  <b>{r.n}</b>
                </li>
              ))}
            </ul>
          </motion.section>
        </div>
      </div>
    </motion.div>
  )
}
