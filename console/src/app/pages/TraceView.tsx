import { motion } from 'motion/react'
import { store } from '../api/store'
import type { Effect, Entry, Finding, Manifest } from '../api/types'
import { EFFECT_GLYPH, EFFECT_LABEL, amount, lamports, named, short } from '../api/format'
import { DecisionStamp, SevChip, SevCounts } from '../ui/Decision'
import Copy from '../ui/Copy'
import Meter from '../ui/Meter'
import Scramble from '../../components/Scramble'
import Storyline from '../ui/Storyline'
import { useReduced } from '../../lib/useReduced'

/* How an effect sits against the tool's manifest. */
type Fit = 'required' | 'allowed' | 'forbidden' | 'undeclared' | 'unchecked'
function fit(e: Effect, m: Manifest | null): Fit {
  if (!m) return 'unchecked'
  if (m.mustNot.includes(e.type)) return 'forbidden'
  if (m.must.includes(e.type)) return 'required'
  if (m.may.includes(e.type)) return 'allowed'
  return 'undeclared'
}
const FIT_LABEL: Record<Fit, string> = {
  required: 'declared · must',
  allowed: 'declared · may',
  forbidden: 'must not',
  undeclared: 'not declared',
  unchecked: 'no manifest',
}
const GROUPS: { k: Finding['group']; title: string; hint: string }[] = [
  { k: 'does', title: 'What it does', hint: 'effects and who they reach' },
  { k: 'compare', title: 'Against the manifest', hint: 'promise vs. behaviour' },
  { k: 'read', title: 'How well it could read', hint: 'coverage of the simulation' },
]

export default function TraceView({ id }: { id: string }) {
  const e = store.get(id)
  if (!e) {
    return (
      <div className="page">
        <p className="kicker">action not found</p>
        <h1>That verdict is not in this session.</h1>
        <a className="link" href="#/actions">back to actions →</a>
      </div>
    )
  }
  return <Trace e={e} />
}

function Trace({ e }: { e: Entry }) {
  const t = e.trace
  const m = t.toolCall?.manifest ?? null
  const reduced = useReduced()
  const d = t.decision.outcome
  const tx = t.transaction
  const cov = t.coverage
  const nodes = [
    { k: 'input', v: e.request.provenance.replace('_', ' '), sub: `“${t.input.goal}”` },
    { k: 'declared', v: t.toolCall?.name ?? '—', sub: m ? `manifest v${m.version}` : 'no manifest' },
    { k: 'simulated', v: tx.simulation, sub: tx.slot ? `slot ${tx.slot.toLocaleString('en-US')}` : '—' },
    { k: 'compared', v: `${t.findings.length} finding${t.findings.length === 1 ? '' : 's'}`, sub: `${t.effects.length} effects` },
    { k: 'decided', v: d, sub: t.policyVersion },
  ]

  return (
    <div className={`page trace trace--${d}`}>
      <a className="link link--muted back" href="#/actions">← actions</a>

      <header className="trace__head">
        <div>
          <Scramble as="p" text={`action ${e.id} · ${e.source === 'live' ? 'live engine' : 'recorded snapshot'}`} className="kicker" />
          <h1><Scramble text={t.toolCall?.name ?? e.request.fixture} hover={false} /></h1>
          <div className="trace__meta">
            <span>digest <Copy value={tx.messageDigest ?? ''} display={short(tx.messageDigest, 8)} /></span>
            <span>policy <code>{t.policyVersion}</code></span>
            <span>fee <code>{lamports(tx.fee)}</code></span>
            {e.source === 'live' && <span>round trip <code>{e.ms} ms</code></span>}
          </div>
        </div>
        <DecisionStamp d={d} />
      </header>

      <ol className="flow" aria-label="Pipeline">
        {nodes.map((n, i) => (
          <motion.li
            key={n.k}
            className={`flow__node ${i === nodes.length - 1 ? `is-last flow__node--${d}` : ''}`}
            initial={reduced ? false : { opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: 0.08 + i * 0.09, type: 'spring', stiffness: 380, damping: 30 }}
          >
            <span className="flow__k">{String(i + 1).padStart(2, '0')} {n.k}</span>
            <b>{n.v}</b>
            <small title={n.sub}>{n.sub}</small>
            {i < nodes.length - 1 && (
              <motion.span
                className="flow__wire"
                aria-hidden="true"
                initial={reduced ? false : { clipPath: 'inset(0 100% 0 0)' }}
                animate={{ clipPath: 'inset(0 0% 0 0)' }}
                transition={{ delay: 0.2 + i * 0.09, duration: 0.35 }}
              >──▶</motion.span>
            )}
          </motion.li>
        ))}
      </ol>

      <section className="panel trace__story">
        <div className="panel__head">
          <h2>What happened</h2>
          <span className="mono-dim">read top to bottom</span>
        </div>
        <Storyline e={e} />
      </section>

      <div className="trace__grid">
        <div className="stack">
          <section className="panel">
            <div className="panel__head">
              <h2>What the transaction does</h2>
              <span className="mono-dim">{t.effects.length} effects · simulated</span>
            </div>
            {tx.simulation !== 'executed' && (
              <p className="callout">
                Simulation <b>{tx.simulation}</b>{tx.simulationDetail ? <> · <code>{tx.simulationDetail}</code></> : null}.
                A transaction that fails has no after-state to read.
              </p>
            )}
            <ul className="effects">
              {t.effects.map((ef, i) => {
                const f = fit(ef, m)
                return (
                  <motion.li
                    key={i}
                    className={`effect fit--${f}`}
                    initial={reduced ? false : { opacity: 0, x: -8 }}
                    animate={{ opacity: 1, x: 0 }}
                    transition={{ delay: 0.35 + i * 0.05 }}
                  >
                    <span className="effect__g" aria-hidden="true">{EFFECT_GLYPH[ef.type]}</span>
                    <span className="effect__main">
                      <b>{EFFECT_LABEL[ef.type]}</b>
                      <span className="mono-dim">
                        {ef.type === 'PROGRAM_INVOKE' ? named(ef.program) : named(ef.account)}
                        {ef.counterparty && <> → <Copy value={ef.counterparty} display={short(ef.counterparty)} /></>}
                      </span>
                    </span>
                    <span className="effect__amt">{amount(ef) ?? ''}</span>
                    <span className={`fit fit--${f}`}>{FIT_LABEL[f]}</span>
                  </motion.li>
                )
              })}
              {!t.effects.length && <li className="empty">No effects could be read.</li>}
            </ul>
          </section>

          <section className="panel">
            <div className="panel__head"><h2>Findings</h2><SevCounts counts={t.decision.counts} /></div>
            <div className="fgroups">
              {GROUPS.map((g) => {
                const list = t.findings.filter((f) => f.group === g.k)
                return (
                  <div key={g.k} className="fgroup">
                    <p className="fgroup__t">{g.title} <span>{g.hint}</span></p>
                    {list.length ? list.map((f, i) => (
                      <details key={i} className={`finding sevline--${f.severity}`}>
                        <summary><SevChip s={f.severity} /><code>{f.rule}</code><span className="mono-dim">{f.check}</span></summary>
                        <pre>{JSON.stringify(f.detail, null, 2)}</pre>
                      </details>
                    )) : <p className="none">nothing here</p>}
                  </div>
                )
              })}
            </div>
          </section>

          {t.explanation && (
            <section className="panel">
              <div className="panel__head"><h2>Engine explanation</h2><span className="mono-dim">pt-BR, as the engine wrote it</span></div>
              <div className="explain" lang="pt-BR">
                <p className="explain__sum">{t.explanation.summary}</p>
                {!!t.explanation.reasons.length && (
                  <>
                    <p className="explain__t">por que</p>
                    <ul>{t.explanation.reasons.map((r, i) => <li key={i}>{r}</li>)}</ul>
                  </>
                )}
                <p className="explain__t">o que faz</p>
                <ul>{t.explanation.does.map((r, i) => <li key={i}>{r}</li>)}</ul>
                {!!t.explanation.minor.length && (
                  <>
                    <p className="explain__t">menores</p>
                    <ul className="dimlist">{t.explanation.minor.map((r, i) => <li key={i}>{r}</li>)}</ul>
                  </>
                )}
              </div>
            </section>
          )}
        </div>

        <div className="stack">
          <section className="panel">
            <div className="panel__head"><h2>Decision ladder</h2><span className="mono-dim">first rule that holds wins</span></div>
            <ol className="ladder">
              {t.decision.steps.map((s, i) => {
                const firstHeld = t.decision.steps.findIndex((x) => x.held)
                const state = i === firstHeld ? 'win' : s.held ? 'held' : i < firstHeld ? 'pass' : 'skip'
                return (
                  <motion.li
                    key={i}
                    className={`ladder__step is-${state}`}
                    initial={reduced ? false : { opacity: 0 }}
                    animate={{ opacity: 1 }}
                    transition={{ delay: 0.3 + i * 0.12 }}
                  >
                    <span className="ladder__mark" aria-hidden="true">{state === 'win' ? '▶' : s.held ? '■' : '·'}</span>
                    <span className="ladder__rule">{s.rule}</span>
                    <span className="ladder__d">{s.detail}</span>
                  </motion.li>
                )
              })}
            </ol>
            <p className="thresholds">
              thresholds · {t.decision.thresholds.highsForDeny} high → deny · {t.decision.thresholds.mediumsForFlag} medium → flag ·
              fee ceiling {lamports(t.decision.thresholds.maxFeeLamports)}
            </p>
          </section>

          <section className="panel">
            <div className="panel__head"><h2>Counterparties</h2><span className="mono-dim">where each address came from</span></div>
            {t.counterparties.length ? (
              <ul className="cps">
                {t.counterparties.map((c) => (
                  <li key={c.address}>
                    <Copy value={c.address} display={short(c.address, 6)} />
                    <span className={`origin origin--${c.origin}`}>{c.origin.replace('_', ' ')}</span>
                    {c.severity && <SevChip s={c.severity} />}
                  </li>
                ))}
              </ul>
            ) : <p className="none">No outside address receives anything.</p>}
          </section>

          <section className="panel">
            <div className="panel__head"><h2>Manifest</h2><span className="mono-dim">{m ? `${m.tool} v${m.version} · ${m.accountScope}` : 'none for this tool'}</span></div>
            {m ? (
              <div className="manifest">
                {(['must', 'may', 'mustNot'] as const).map((k) => (
                  <div key={k} className="manifest__row">
                    <span className="manifest__k">{k === 'mustNot' ? 'must not' : k}</span>
                    <span className="manifest__chips">
                      {m[k].map((x) => (
                        <span key={x} className={`mchip mchip--${k} ${t.effects.some((ef) => ef.type === x) ? 'is-seen' : ''}`}>{EFFECT_GLYPH[x]} {x}</span>
                      ))}
                    </span>
                  </div>
                ))}
                <p className="mono-dim small">highlighted = present in this transaction</p>
              </div>
            ) : <p className="none">Without a manifest the engine cannot compare promise with behaviour; provenance and the safety net still apply.</p>}
          </section>

          <section className="panel">
            <div className="panel__head"><h2>Transaction</h2></div>
            <dl className="kv">
              <dt>digest</dt><dd><Copy value={tx.messageDigest ?? ''} display={short(tx.messageDigest, 10)} /></dd>
              <dt>slot</dt><dd>{tx.slot?.toLocaleString('en-US') ?? '—'}{tx.baselineSlot != null && tx.baselineSlot !== tx.slot && <span className="mono-dim"> · baseline {tx.baselineSlot.toLocaleString('en-US')}</span>}</dd>
              <dt>fee payer</dt><dd>{tx.feePayer ? <Copy value={tx.feePayer} display={short(tx.feePayer, 6)} /> : '—'}</dd>
              <dt>fee</dt><dd>{lamports(tx.fee)}</dd>
              <dt>simulation</dt><dd className={`sim sim--${tx.simulation}`}>{tx.simulation}</dd>
            </dl>
            <div className="coverage">
              <span>accounts observed</span>
              <Meter value={cov.referenced ? cov.observed / cov.referenced : 0} cols={20} />
              <b>{cov.observed}/{cov.referenced}</b>
            </div>
            <p className="mono-dim small">
              {cov.blindSpots.length + cov.opaqueChanges.length + cov.undecodable.length
                ? `${cov.blindSpots.length} blind spots · ${cov.opaqueChanges.length} opaque changes · ${cov.undecodable.length} undecodable`
                : 'no blind spots, opaque changes or undecodable instructions'}
            </p>
          </section>
        </div>
      </div>
    </div>
  )
}
