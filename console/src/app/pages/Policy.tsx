import { useEntries } from '../api/store'
import { config, useConfig } from '../config/store'
import { decide, gateOutcome, impact } from '../config/impact'
import { Choice, PageHead, Row, Stepper, Switch } from '../config/ui'
import { DecisionBadge } from '../ui/Decision'
import { lamports } from '../api/format'
import Counter from '../../components/Counter'

export default function Policy() {
  const s = useConfig()
  const d = s.draft, p = s.history[0].config
  const entries = useEntries()
  const imp = impact(entries, p, d)
  const ladder = [
    { rule: 'any critical', to: 'deny' as const },
    { rule: `${d.thresholds.highsForDeny}+ high`, to: 'deny' as const },
    { rule: 'any high', to: 'flag' as const },
    { rule: `${d.thresholds.mediumsForFlag}+ medium`, to: 'flag' as const },
    { rule: 'otherwise', to: 'allow' as const },
  ]
  const feeSol = Number(d.thresholds.maxFeeLamports) / 1e9

  return (
    <div className="page">
      <PageHead
        kicker="configure · decision policy"
        title="How verdicts are decided"
        lede="Findings are never scored. They are counted by severity and walked down a fixed ladder; these numbers move the rungs. Changes stay in the draft until you publish."
      />
      <div className="cfggrid">
        <div className="stack">
          <section className="panel">
            <div className="panel__head"><h2>Thresholds</h2><span className="mono-dim">Thresholds · DEFAULT 2 / 1 / 0.01 SOL</span></div>
            <Row label="Highs that deny" hint="How many high-severity findings turn a flag into a deny." changed={d.thresholds.highsForDeny !== p.thresholds.highsForDeny}>
              <Stepper id="highs" value={d.thresholds.highsForDeny} min={1} max={9} onChange={(v) => config.edit((c) => { c.thresholds.highsForDeny = v })} />
            </Row>
            <Row label="Mediums that flag" hint="How many medium findings hold a transaction for a person." changed={d.thresholds.mediumsForFlag !== p.thresholds.mediumsForFlag}>
              <Stepper id="mediums" value={d.thresholds.mediumsForFlag} min={1} max={9} onChange={(v) => config.edit((c) => { c.thresholds.mediumsForFlag = v })} />
            </Row>
            <Row label="Fee ceiling" hint={`Above this the engine raises magnitude.fee_above_ceiling. Now ${lamports(d.thresholds.maxFeeLamports)}.`} changed={d.thresholds.maxFeeLamports !== p.thresholds.maxFeeLamports}>
              <Stepper id="fee" value={Math.round(feeSol * 1000)} min={1} max={1000} unit="mSOL" onChange={(v) => config.edit((c) => { c.thresholds.maxFeeLamports = String(v * 1_000_000) })} />
            </Row>
          </section>

          <section className="panel">
            <div className="panel__head"><h2>Gate</h2><span className="mono-dim">what signing does with each decision</span></div>
            <Row label="When a verdict is FLAG" hint="Deny is always refused and allow always signed. Flag is the operator's call." changed={d.gate.onFlag !== p.gate.onFlag}>
              <Choice id="onflag" value={d.gate.onFlag} options={[{ v: 'sign', label: 'sign' }, { v: 'hold', label: 'hold' }, { v: 'refuse', label: 'refuse' }]} onChange={(v) => config.edit((c) => { c.gate.onFlag = v })} />
            </Row>
            <Row label="Authorization lifetime" hint="A signature permit is single-use and bound to the message digest; this is how long it stays valid." changed={d.gate.authorizationTtlSec !== p.gate.authorizationTtlSec}>
              <Stepper id="ttl" value={d.gate.authorizationTtlSec} min={5} max={600} step={5} unit="s" onChange={(v) => config.edit((c) => { c.gate.authorizationTtlSec = v })} />
            </Row>
          </section>

          <section className="panel">
            <div className="panel__head"><h2>Judge & deadlines</h2></div>
            <Row label="LLM judge" hint="A model reviews the deterministic verdict. It can only make it stricter, never looser." changed={d.judge.enabled !== p.judge.enabled}>
              <Switch label="LLM judge" on={d.judge.enabled} onChange={(v) => config.edit((c) => { c.judge.enabled = v })} />
            </Row>
            <Row label="Account read" hint="Budget to fetch the accounts a transaction touches." changed={d.deadlines.readMs !== p.deadlines.readMs}>
              <Stepper id="read" value={d.deadlines.readMs} min={250} max={10000} step={250} unit="ms" onChange={(v) => config.edit((c) => { c.deadlines.readMs = v })} />
            </Row>
            <Row label="Simulation" hint="Budget for the simulation itself (aval-svm locally, RPC as fallback)." changed={d.deadlines.simulateMs !== p.deadlines.simulateMs}>
              <Stepper id="sim" value={d.deadlines.simulateMs} min={250} max={10000} step={250} unit="ms" onChange={(v) => config.edit((c) => { c.deadlines.simulateMs = v })} />
            </Row>
            <Row label="Total" hint="Past this the verdict is coverage.simulation_unavailable, never a silent allow." changed={d.deadlines.totalMs !== p.deadlines.totalMs}>
              <Stepper id="total" value={d.deadlines.totalMs} min={500} max={20000} step={500} unit="ms" onChange={(v) => config.edit((c) => { c.deadlines.totalMs = v })} />
            </Row>
            <Row label="Network" hint="Which cluster the Scenario Lab simulates against." live changed={d.network !== p.network}>
              <Choice id="net" value={d.network} options={[{ v: 'mainnet', label: 'mainnet' }, { v: 'devnet', label: 'devnet' }]} onChange={(v) => config.edit((c) => { c.network = v })} />
            </Row>
          </section>
        </div>

        <aside className="stack sticky">
          <section className="panel">
            <div className="panel__head"><h2>Ladder · draft</h2><span className="mono-dim">first rung that holds wins</span></div>
            <ol className="ladder ladder--cfg">
              {ladder.map((r, i) => (
                <li key={i} className="ladder__step is-pass">
                  <span className="ladder__mark" aria-hidden="true">{i + 1}</span>
                  <span className="ladder__rule">{r.rule}</span>
                  <span><DecisionBadge d={r.to} /> <small className="mono-dim">→ {gateOutcome(r.to, d.gate.onFlag)}</small></span>
                </li>
              ))}
            </ol>
          </section>
          <section className="panel impact">
            <div className="panel__head"><h2>Impact on this session</h2><span className="mono-dim">{imp.considered} real verdicts replayed</span></div>
            <div className="impact__nums">
              <div><Counter to={imp.flips.length} className="impact__n" /><span>verdicts change</span></div>
              <div><Counter to={imp.gate.length} className="impact__n" /><span>signing outcomes change</span></div>
              <div><Counter to={imp.overFee.length} className="impact__n" /><span>over fee ceiling</span></div>
            </div>
            <ul className="flips">
              {imp.flips.slice(0, 6).map((f) => (
                <li key={f.entry.id}>
                  <code>{f.entry.trace.toolCall?.name ?? f.entry.request.fixture}</code>
                  <span><DecisionBadge d={f.from} /> → <DecisionBadge d={f.to} /></span>
                </li>
              ))}
              {!imp.flips.length && <li className="none">No verdict in this session changes under the draft.</li>}
            </ul>
            <p className="mono-dim small">Replayed with the engine's ladder on each verdict's real severity counts. Session counts by today's rules: {(['deny', 'flag', 'allow'] as const).map((x) => `${entries.filter((e) => decide(e.trace.decision.counts, p.thresholds) === x).length} ${x}`).join(' · ')}.</p>
          </section>
        </aside>
      </div>
    </div>
  )
}
