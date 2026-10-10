import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from 'react'
import { motion } from 'motion/react'
import { store, useEntries } from '../api/store'
import type { Effect, Entry, Finding, Severity, Trace } from '../api/types'
import { EFFECT_GLYPH, EFFECT_LABEL, amount, lamports, named, short } from '../api/format'
import { config, useConfig, type ManifestCfg, type ListKey } from '../config/store'
import { classOf, decide, gateOutcome } from '../config/impact'
import { DecisionBadge, SevChip } from '../ui/Decision'
import Copy from '../ui/Copy'
import { toast } from '../ui/Toasts'
import { go } from '../router'
import { useReduced } from '../../lib/useReduced'

/*
 * Forensic view of one intervention, reached from the Overview. Everything
 * here is read from the trace the engine returned (or recomputed with the
 * engine's own rules, and labelled as such). Hovering any address lights it up
 * in every section, so one counterparty can be followed end to end.
 */

const SEV_RANK: Record<Severity, number> = { critical: 0, high: 1, medium: 2, low: 3 }
const SECTIONS = [
  ['decision', 'Decision'],
  ['provenance', 'Input & provenance'],
  ['declared', 'Declared vs executed'],
  ['effects', 'Effects ledger'],
  ['addresses', 'Address book'],
  ['coverage', 'Coverage & simulation'],
  ['explain', 'Engine explanation'],
  ['raw', 'Raw trace'],
] as const

/* ── address focus shared across sections ─────────────────────────── */
const Focus = createContext<{ focus: string | null; set: (a: string | null) => void }>({ focus: null, set: () => {} })
function Addr({ a, n = 4, label }: { a: string | null | undefined; n?: number; label?: string }) {
  const { focus, set } = useContext(Focus)
  if (!a) return <span className="mono-dim">—</span>
  return (
    <span className={`addr ${focus === a ? 'is-focus' : ''}`} onMouseEnter={() => set(a)} onMouseLeave={() => set(null)} onFocus={() => set(a)} onBlur={() => set(null)}>
      <Copy value={a} display={label ?? (named(a) !== short(a) ? named(a) : short(a, n))} />
    </span>
  )
}

/* ── helpers ───────────────────────────────────────────────────────── */
function argValue(ref: unknown, args: Record<string, unknown>) {
  if (typeof ref === 'string' && ref.startsWith('arg:')) return { label: ref, value: args[ref.slice(4)] == null ? null : String(args[ref.slice(4)]) }
  return { label: 'literal', value: ref == null ? null : String(ref) }
}
/** A declared decimal and a raw on-chain amount agree if raw = decimal × 10^k for a common k. */
function amountsAgree(decl: string, raw: string) {
  const [i, f = ''] = decl.split('.')
  for (const k of [0, 6, 8, 9]) {
    if (f.length > k) continue
    const scaled = (i + f.padEnd(k, '0')).replace(/^0+(?=\d)/, '')
    if (scaled === raw.replace(/^0+(?=\d)/, '')) return true
  }
  return false
}
type Check = { field: string; ref: string; expected: string | null; actual: string | null; ok: boolean | null }
function checkPattern(p: Record<string, unknown>, ef: Effect | undefined, args: Record<string, unknown>): Check[] {
  const out: Check[] = []
  for (const [k, v] of Object.entries(p)) {
    if (['type', 'unit', 'tolerance'].includes(k)) continue
    const { label, value } = argValue(v, args)
    const field = k === 'maxAmount' ? 'amount' : k
    const actual = ef ? ((ef as unknown as Record<string, unknown>)[field] as string | null) ?? null : null
    let ok: boolean | null = null
    if (value != null && actual != null) {
      if (k === 'maxAmount') ok = amountsAgree(value, actual) || Number(actual) <= Number(value)
      else if (value === 'SOL' || actual === 'SOL') ok = value === actual
      else ok = value === actual
    }
    out.push({ field: k, ref: label, expected: value, actual, ok })
  }
  return out
}
function addressesIn(text: string) { return [...new Set(text.match(/[1-9A-HJ-NP-Za-km-z]{32,44}/g) ?? [])] }

/* ── page ──────────────────────────────────────────────────────────── */
export default function Inspect({ id }: { id: string }) {
  const entries = useEntries()
  const e = store.get(id)
  const interventions = entries.filter((x) => x.trace.decision.outcome !== 'allow')
  const idx = interventions.findIndex((x) => x.id === id)
  const prev = idx > 0 ? interventions[idx - 1] : null
  const next = idx >= 0 && idx < interventions.length - 1 ? interventions[idx + 1] : null
  const [focus, set] = useState<string | null>(null)

  useEffect(() => {
    const key = (ev: KeyboardEvent) => {
      if ((ev.target as HTMLElement)?.closest('input, textarea, select')) return
      if (ev.key === '[' && prev) go(`overview/inspect/${prev.id}`)
      if (ev.key === ']' && next) go(`overview/inspect/${next.id}`)
      if (ev.key === 'Escape') go('overview')
    }
    addEventListener('keydown', key)
    return () => removeEventListener('keydown', key)
  }, [prev?.id, next?.id])

  if (!e) {
    return (
      <div className="page">
        <nav className="crumbs"><a href="#/overview">Overview</a><span>/</span><span>inspect</span></nav>
        <h1>That action is not in this session.</h1>
      </div>
    )
  }

  return (
    <Focus.Provider value={{ focus, set }}>
      <div className="page insp">
        <nav className="crumbs" aria-label="Breadcrumb">
          <a href="#/overview">Overview</a><span>/</span><span>inspect</span><span>/</span><span className="ink">{e.id}</span>
          <span className="crumbs__nav">
            <button type="button" className="act act--ghost" disabled={!prev} onClick={() => prev && go(`overview/inspect/${prev.id}`)}>[ prev</button>
            <span className="mono-dim small">{idx >= 0 ? `${idx + 1} of ${interventions.length} interventions` : 'not an intervention'}</span>
            <button type="button" className="act act--ghost" disabled={!next} onClick={() => next && go(`overview/inspect/${next.id}`)}>next ]</button>
          </span>
        </nav>
        <Body e={e} />
      </div>
    </Focus.Provider>
  )
}

function Body({ e }: { e: Entry }) {
  const t = e.trace
  const cfg = useConfig()
  const reduced = useReduced()
  const d = t.decision.outcome
  const manifest: ManifestCfg | undefined = cfg.history[0].config.manifests.find((m) => m.tool === t.toolCall?.name)
  const args = (t.toolCall?.args ?? {}) as Record<string, unknown>
  const [active, setActive] = useState<string>('decision')

  useEffect(() => {
    const io = new IntersectionObserver((xs) => {
      const v = xs.filter((x) => x.isIntersecting).sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top)[0]
      if (v) setActive(v.target.id)
    }, { rootMargin: '-20% 0px -70% 0px' })
    SECTIONS.forEach(([id]) => { const el = document.getElementById(id); if (el) io.observe(el) })
    return () => io.disconnect()
  }, [e.id])

  return (
    <>
      <header className="insp__head">
        <div className="insp__title">
          <p className="kicker">{e.source === 'live' ? 'live engine' : 'recorded snapshot'} · {new Date(e.at).toLocaleString('en-US', { hour12: false })}</p>
          <h1>{t.toolCall?.name ?? e.request.fixture}</h1>
          <p className="mono-dim">“{t.input.goal}”</p>
        </div>
        <div className={`insp__verdict insp__verdict--${d}`}>
          <span className="insp__vword">{d.toUpperCase()}</span>
          <span className="small">gate: {gateOutcome(d, cfg.history[0].config.gate.onFlag)}</span>
        </div>
      </header>

      <dl className="insp__facts">
        <div><dt>digest</dt><dd><Copy value={t.transaction.messageDigest ?? ''} display={short(t.transaction.messageDigest, 8)} /></dd></div>
        <div><dt>slot</dt><dd>{t.transaction.slot?.toLocaleString('en-US') ?? '—'}</dd></div>
        <div><dt>baseline</dt><dd>{t.transaction.baselineSlot?.toLocaleString('en-US') ?? '—'}{t.transaction.slot && t.transaction.baselineSlot ? <span className="mono-dim"> ({t.transaction.slot - t.transaction.baselineSlot} gap)</span> : null}</dd></div>
        <div><dt>simulation</dt><dd className={`sim sim--${t.transaction.simulation}`}>{t.transaction.simulation}</dd></div>
        <div><dt>fee</dt><dd>{lamports(t.transaction.fee)}</dd></div>
        <div><dt>fee payer</dt><dd><Addr a={t.transaction.feePayer} n={5} /></dd></div>
        <div><dt>policy</dt><dd><code>{t.policyVersion}</code></dd></div>
        <div><dt>declared by</dt><dd>{t.declarationSource}</dd></div>
        <div><dt>provenance</dt><dd>{e.request.provenance.replace('_', ' ')}</dd></div>
        <div><dt>round trip</dt><dd>{e.source === 'live' ? `${e.ms} ms` : '—'}</dd></div>
      </dl>

      <div className="insp__grid">
        <nav className="insp__toc" aria-label="Sections">
          {SECTIONS.map(([id, label]) => (
            <a key={id} href={`#/overview/inspect/${e.id}`} className={active === id ? 'is-on' : ''}
              onClick={(ev) => { ev.preventDefault(); document.getElementById(id)?.scrollIntoView({ behavior: reduced ? 'auto' : 'smooth', block: 'start' }) }}>
              {active === id && <motion.span layoutId="toc-hl" className="insp__tocbar" transition={{ type: 'spring', stiffness: 500, damping: 40 }} />}
              {label}
            </a>
          ))}
          <p className="mono-dim small insp__keys">[ ] previous / next<br />esc back to overview</p>
        </nav>

        <div className="insp__main">
          <DecisionSection t={t} />
          <ProvenanceSection e={e} />
          <DeclaredSection t={t} manifest={manifest} args={args} />
          <EffectsSection t={t} manifest={manifest} />
          <AddressSection t={t} args={args} />
          <CoverageSection t={t} />
          <ExplainSection t={t} />
          <RawSection t={t} />
        </div>
      </div>
    </>
  )
}

function Section({ id, title, note, children }: { id: string; title: string; note?: ReactNode; children: ReactNode }) {
  return (
    <section id={id} className="isec">
      <header className="isec__head"><h2>{title}</h2>{note && <span className="mono-dim small">{note}</span>}</header>
      {children}
    </section>
  )
}

/* ── 1. decision ───────────────────────────────────────────────────── */
function DecisionSection({ t }: { t: Trace }) {
  const cfg = useConfig()
  const th = t.decision.thresholds
  const c = t.decision.counts
  const findings = [...t.findings].sort((a, b) => SEV_RANK[a.severity] - SEV_RANK[b.severity])
  const firstHeld = t.decision.steps.findIndex((s) => s.held)
  const thNum = { highsForDeny: th.highsForDeny, mediumsForFlag: th.mediumsForFlag, maxFeeLamports: th.maxFeeLamports }
  // Counterfactual: remove one finding, recompute with the engine's ladder.
  const cf = findings.map((f) => {
    const cc = { ...c, [f.severity]: c[f.severity] - 1 }
    return { f, to: decide(cc, thNum) }
  })
  const draftTh = cfg.draft.thresholds
  const underDraft = decide(c, draftTh)

  return (
    <Section id="decision" title="Decision" note="severity counts walked down the engine's ladder">
      <div className="dgrid">
        <div>
          <div className="counts">
            {(['critical', 'high', 'medium', 'low'] as Severity[]).map((s) => (
              <div key={s} className={`count count--${s} ${c[s] ? '' : 'is-zero'}`}>
                <b>{c[s]}</b><span>{s}</span>
              </div>
            ))}
          </div>
          <ol className="ladder ladder--insp">
            {t.decision.steps.map((s, i) => (
              <li key={i} className={`ladder__step ${i === firstHeld ? 'is-win' : s.held ? 'is-held' : i < firstHeld ? 'is-pass' : 'is-skip'}`}>
                <span className="ladder__mark" aria-hidden="true">{i === firstHeld ? '▶' : i < firstHeld ? '✓' : '·'}</span>
                <span className="ladder__rule">{s.rule}</span>
                <span className="ladder__d">{i < firstHeld ? 'did not hold' : i === firstHeld ? 'held → decided' : 'not reached'} · {s.detail}</span>
              </li>
            ))}
          </ol>
          <p className="mono-dim small">thresholds at decision time: {th.highsForDeny} high → deny · {th.mediumsForFlag} medium → flag · fee ceiling {lamports(th.maxFeeLamports)}</p>
        </div>
        <div>
          <p className="vi__lbl">which findings decided it <em className="mono-dim">· console recompute</em></p>
          <ul className="cf">
            {cf.map(({ f, to }, i) => {
              const decisive = to !== t.decision.outcome
              return (
                <li key={i} className={decisive ? 'is-decisive' : ''}>
                  <SevChip s={f.severity} />
                  <code>{f.rule}</code>
                  <span className="cf__arrow">without it →</span>
                  <DecisionBadge d={to} />
                  {decisive && <span className="cf__tag">decisive</span>}
                </li>
              )
            })}
            {!cf.length && <li className="none">No findings. Allowed by default.</li>}
          </ul>
          <p className="mono-dim small cf__draft">
            under the current draft policy ({draftTh.highsForDeny}/{draftTh.mediumsForFlag}) this would be <DecisionBadge d={underDraft} />
            {underDraft !== t.decision.outcome && <b className="ink"> · changes</b>}
          </p>
        </div>
      </div>
      <div className="findings">
        {findings.map((f, i) => <FindingRow key={i} f={f} />)}
      </div>
    </Section>
  )
}

function FindingRow({ f }: { f: Finding }) {
  const addrs = addressesIn(JSON.stringify(f.detail))
  return (
    <details className={`frow sevline--${f.severity}`} open={f.severity === 'critical' || f.severity === 'high'}>
      <summary>
        <SevChip s={f.severity} />
        <code className="frow__rule">{f.rule}</code>
        <span className="mono-dim small">check {f.check} · group {f.group}</span>
        <span className="frow__addrs">{addrs.slice(0, 2).map((a) => <Addr key={a} a={a} />)}</span>
      </summary>
      <div className="frow__body">
        {Object.entries(f.detail).map(([k, v]) => (
          <div key={k} className="kvrow">
            <span className="kvrow__k">{k}</span>
            <span className="kvrow__v">{typeof v === 'string' && /^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(v) ? <Addr a={v} n={8} /> : Array.isArray(v) ? (v.length ? v.map((x) => JSON.stringify(x)).join(', ') : <span className="mono-dim">[] empty</span>) : typeof v === 'object' && v ? JSON.stringify(v) : String(v)}</span>
          </div>
        ))}
      </div>
    </details>
  )
}

/* ── 2. provenance ─────────────────────────────────────────────────── */
function ProvenanceSection({ e }: { e: Entry }) {
  const t = e.trace
  const req = e.request
  const read = req.retrieved ?? ''
  const cpAddrs = new Set(t.counterparties.map((c) => c.address))
  return (
    <Section id="provenance" title="Input & provenance" note="what the agent was told, and where every address came from">
      <div className="chunks">
        {t.input.chunks.map((ch, i) => (
          <div key={i} className={`chunk ${ch.trusted ? 'is-trusted' : 'is-untrusted'}`}>
            <div className="chunk__head">
              <span className={`trust ${ch.trusted ? 'trust--y' : 'trust--n'}`}>{ch.trusted ? 'trusted' : 'untrusted'}</span>
              <span className="mono-dim small">provenance {ch.provenance}{ch.source ? ` · ${ch.source}` : ''}</span>
            </div>
            <p className="chunk__text"><Highlighted text={ch.text} hot={cpAddrs} /></p>
          </div>
        ))}
        {read && !t.input.chunks.some((c) => c.text.includes(read.slice(0, 20))) && (
          <div className="chunk is-untrusted">
            <div className="chunk__head"><span className="trust trust--n">sent as context</span><span className="mono-dim small">{req.source ?? 'no source given'}</span></div>
            <p className="chunk__text"><Highlighted text={read} hot={cpAddrs} /></p>
          </div>
        )}
      </div>
      <div className="itable-wrap">
      <table className="itable">
        <thead><tr><th>counterparty</th><th>origin</th><th>sources</th><th>severity</th></tr></thead>
        <tbody>
          {t.counterparties.map((c) => (
            <tr key={c.address}>
              <td><Addr a={c.address} n={8} /></td>
              <td><span className={`origin origin--${c.origin}`}>{c.origin.replace('_', ' ')}</span></td>
              <td className="mono-dim">{c.sources.length ? c.sources.join(', ') : 'none: not found in anything the user wrote or the agent read'}</td>
              <td>{c.severity ? <SevChip s={c.severity} /> : <span className="mono-dim">none</span>}</td>
            </tr>
          ))}
          {!t.counterparties.length && <tr><td colSpan={4} className="none">No outside address receives anything.</td></tr>}
        </tbody>
      </table>
      </div>
    </Section>
  )
}
function Highlighted({ text, hot }: { text: string; hot: Set<string> }) {
  const parts = text.split(/([1-9A-HJ-NP-Za-km-z]{32,44})/g)
  return <>{parts.map((p, i) => (/^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(p) ? <mark key={i} className={hot.has(p) ? 'is-hot' : ''}><Addr a={p} n={6} /></mark> : <span key={i}>{p}</span>))}</>
}

/* ── 3. declared vs executed ───────────────────────────────────────── */
function DeclaredSection({ t, manifest, args }: { t: Trace; manifest?: ManifestCfg; args: Record<string, unknown> }) {
  return (
    <Section id="declared" title="Declared vs executed" note={manifest ? `${manifest.tool} manifest v${manifest.version} · scope ${manifest.accountScope}` : 'no manifest for this tool'}>
      <div className="dgrid">
        <div>
          <p className="vi__lbl">tool call arguments</p>
          <div className="itable-wrap">
          <table className="itable itable--kv">
            <tbody>
              {Object.entries(args).map(([k, v]) => (
                <tr key={k}><td className="mono-dim">{k}</td><td>{typeof v === 'string' && /^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(v) ? <Addr a={v} n={8} /> : <code>{String(v)}</code>}</td>
                  <td className="mono-dim small">{manifest?.args[k] ?? ''}</td></tr>
              ))}
              {!Object.keys(args).length && <tr><td className="none">no arguments recorded</td></tr>}
            </tbody>
          </table>
          </div>
        </div>
        <div>
          <p className="vi__lbl">must-not present?</p>
          <ul className="mustnot">
            {(manifest?.mustNot ?? []).map((x) => {
              const hit = t.effects.some((ef) => ef.type === x)
              return <li key={x} className={hit ? 'is-hit' : ''}><span>{hit ? '✕' : '✓'}</span><code>{x}</code><span className="mono-dim small">{hit ? 'present → violation' : 'absent'}</span></li>
            })}
            {!manifest && <li className="none">Nothing to compare: the engine raises coverage.no_manifest.</li>}
          </ul>
        </div>
      </div>
      {manifest && (
        <div className="pats">
          <p className="vi__lbl">required effects, field by field <em className="mono-dim">· argument refs resolved by the console</em></p>
          {manifest.must.map((p, i) => {
            const ef = t.effects.find((x) => x.type === p.type)
            const checks = checkPattern(p as Record<string, unknown>, ef, args)
            return (
              <div key={i} className={`pat ${ef ? '' : 'is-missing'}`}>
                <div className="pat__head">
                  <span className="vi__glyph">{EFFECT_GLYPH[p.type]}</span><b>{EFFECT_LABEL[p.type]}</b>
                  <span className={`pat__state ${ef ? 'is-ok' : 'is-bad'}`}>{ef ? 'found in simulation' : 'missing: must-effect absent'}</span>
                </div>
                {!!checks.length && (
                  <div className="itable-wrap itable-wrap--flat">
                  <table className="itable itable--pat">
                    <thead><tr><th>field</th><th>declared via</th><th>expected</th><th>simulated</th><th></th></tr></thead>
                    <tbody>
                      {checks.map((c) => (
                        <tr key={c.field} className={c.ok === false ? 'is-bad' : ''}>
                          <td>{c.field}</td><td className="mono-dim">{c.ref}</td>
                          <td>{c.expected && /^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(c.expected) ? <Addr a={c.expected} /> : <code>{c.expected ?? '—'}</code>}</td>
                          <td>{c.actual && /^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(c.actual) ? <Addr a={c.actual} /> : <code>{c.actual ?? '—'}</code>}</td>
                          <td>{c.ok == null ? <span className="mono-dim">n/a</span> : c.ok ? <span className="ok">match</span> : <span className="bad">differs</span>}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                  </div>
                )}
              </div>
            )
          })}
          {!manifest.must.length && <p className="none">This manifest requires no specific effect.</p>}
        </div>
      )}
    </Section>
  )
}

/* ── 4. effects ledger ─────────────────────────────────────────────── */
function EffectsSection({ t, manifest }: { t: Trace; manifest?: ManifestCfg }) {
  const [hideCalls, setHide] = useState(false)
  const rows = t.effects.filter((ef) => !(hideCalls && ef.type === 'PROGRAM_INVOKE'))
  return (
    <Section id="effects" title="Effects ledger" note={<label className="inline"><input type="checkbox" checked={hideCalls} onChange={(ev) => setHide(ev.target.checked)} /> hide program calls</label>}>
      <div className="itable-wrap">
        <table className="itable itable--dense">
          <thead><tr><th>#</th><th>effect</th><th>account</th><th>owner</th><th>mint</th><th>amount</th><th>counterparty</th><th>program</th><th>manifest</th></tr></thead>
          <tbody>
            {rows.map((ef, i) => {
              const fit = manifest ? classOf(manifest, ef.type) : 'none'
              return (
                <tr key={i} className={fit === 'mustNot' ? 'is-bad' : ''}>
                  <td className="mono-dim">{i + 1}</td>
                  <td><span className="vi__glyph">{EFFECT_GLYPH[ef.type]}</span> {ef.type}</td>
                  <td><Addr a={ef.account} /></td>
                  <td><Addr a={ef.owner} /></td>
                  <td>{ef.mint === 'SOL' ? 'SOL' : <Addr a={ef.mint} />}</td>
                  <td title={ef.amount ?? ''}>{amount(ef) ?? <span className="mono-dim">—</span>}{ef.amount && ef.mint !== 'SOL' && <small className="mono-dim"> raw {ef.amount}</small>}</td>
                  <td><Addr a={ef.counterparty} /></td>
                  <td><Addr a={ef.program} /></td>
                  <td><span className={`fit fit--${fit === 'mustNot' ? 'forbidden' : fit === 'must' ? 'required' : fit === 'may' ? 'allowed' : manifest ? 'undeclared' : 'unchecked'}`}>{manifest ? (fit === 'none' ? 'undeclared' : fit) : 'no manifest'}</span></td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
    </Section>
  )
}

/* ── 5. address book ───────────────────────────────────────────────── */
function AddressSection({ t, args }: { t: Trace; args: Record<string, unknown> }) {
  const cfg = useConfig()
  const roles = new Map<string, Set<string>>()
  const add = (a: string | null | undefined, r: string) => { if (!a || a === 'SOL') return; if (!roles.has(a)) roles.set(a, new Set()); roles.get(a)!.add(r) }
  add(t.transaction.feePayer, 'fee payer')
  t.effects.forEach((ef) => { add(ef.account, `${ef.type.toLowerCase()} account`); add(ef.owner, 'owner'); add(ef.counterparty, 'counterparty'); add(ef.program, 'program'); add(ef.mint, 'mint') })
  Object.entries(args).forEach(([k, v]) => typeof v === 'string' && /^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(v) && add(v, `arg:${k}`))
  const cp = new Map(t.counterparties.map((c) => [c.address, c]))
  const listsOf = (a: string, which: 'draft' | 'pub') => {
    const L = which === 'draft' ? cfg.draft.lists : cfg.history[0].config.lists
    return (Object.keys(L) as ListKey[]).filter((k) => L[k].some((x) => x.value === a))
  }
  const stage = (a: string, k: ListKey) => {
    config.edit((c) => { if (!c.lists[k].some((x) => x.value === a)) c.lists[k].unshift({ value: a, note: `from inspection ${short(t.transaction.messageDigest, 4)}`, addedAt: Date.now() }) })
    toast({ title: `${k === 'denylist' ? 'Deny-list' : 'Allowlist'} + ${short(a, 6)}`, body: 'Staged in the draft. Publish it from Deploy.', tone: k === 'denylist' ? 'deny' : 'ink' })
  }
  const rows = [...roles.entries()].sort((a, b) => (cp.has(b[0]) ? 1 : 0) - (cp.has(a[0]) ? 1 : 0))
  return (
    <Section id="addresses" title="Address book" note={`${rows.length} addresses in this transaction`}>
      <div className="itable-wrap">
        <table className="itable">
          <thead><tr><th>address</th><th>roles here</th><th>origin</th><th>lists</th><th></th></tr></thead>
          <tbody>
            {rows.map(([a, rs]) => {
              const c = cp.get(a)
              const draft = listsOf(a, 'draft'), pub = listsOf(a, 'pub')
              const isProgram = rs.has('program') || rs.has('mint')
              return (
                <tr key={a} className={c?.severity === 'critical' ? 'is-bad' : ''}>
                  <td><Addr a={a} n={8} /></td>
                  <td className="small">{[...rs].join(' · ')}</td>
                  <td>{c ? <span className={`origin origin--${c.origin}`}>{c.origin.replace('_', ' ')}</span> : <span className="mono-dim">—</span>}</td>
                  <td className="small">{pub.length ? pub.join(', ') : draft.length ? <span className="draftchip">{draft.join(', ')} (draft)</span> : <span className="mono-dim">none</span>}</td>
                  <td className="nowrap">
                    {!isProgram && !draft.includes('denylist') && <button type="button" className="act act--ghost" onClick={() => stage(a, 'denylist')}>block</button>}
                    {!isProgram && c && !draft.includes('allowlist') && <button type="button" className="act act--ghost" onClick={() => stage(a, 'allowlist')}>trust</button>}
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
    </Section>
  )
}

/* ── 6. coverage ───────────────────────────────────────────────────── */
function CoverageSection({ t }: { t: Trace }) {
  const cov = t.coverage
  const lists: [string, unknown[]][] = [['blind spots', cov.blindSpots], ['token extensions', cov.tokenExtensions], ['opaque changes', cov.opaqueChanges], ['inexact amounts', cov.inexactAmounts], ['undecodable', cov.undecodable], ['over simulation limit', cov.overSimulationLimit]]
  const pct = cov.referenced ? cov.observed / cov.referenced : 0
  return (
    <Section id="coverage" title="Coverage & simulation" note="how much of the transaction the engine could actually see">
      <div className="covbar">
        <div className="covbar__track"><motion.div className="covbar__fill" initial={{ width: 0 }} animate={{ width: `${pct * 100}%` }} transition={{ duration: 0.8, ease: [0.16, 1, 0.3, 1] }} /></div>
        <span>{cov.observed} observed · {cov.unobserved} unobserved · {cov.referenced} referenced</span>
      </div>
      <div className="covgrid">
        {lists.map(([k, v]) => (
          <div key={k} className={`covcell ${v.length ? 'is-bad' : ''}`}><b>{v.length}</b><span>{k}</span></div>
        ))}
      </div>
      {t.transaction.simulationDetail && <p className="callout">simulation detail · <code>{t.transaction.simulationDetail}</code></p>}
    </Section>
  )
}

/* ── 7. explanation ───────────────────────────────────────────────── */
function ExplainSection({ t }: { t: Trace }) {
  const x = t.explanation
  return (
    <Section id="explain" title="Engine explanation" note="pt-BR, written by the engine">
      {x ? (
        <div className="explain" lang="pt-BR">
          <p className="explain__sum">{x.summary}</p>
          {!!x.reasons.length && <><p className="explain__t">por que</p><ul>{x.reasons.map((r, i) => <li key={i}>{r}</li>)}</ul></>}
          <p className="explain__t">o que faz</p><ul>{x.does.map((r, i) => <li key={i}>{r}</li>)}</ul>
          {!!x.minor.length && <><p className="explain__t">menores</p><ul className="dimlist">{x.minor.map((r, i) => <li key={i}>{r}</li>)}</ul></>}
        </div>
      ) : <p className="none">No explanation returned.</p>}
    </Section>
  )
}

/* ── 8. raw ────────────────────────────────────────────────────────── */
function RawSection({ t }: { t: Trace }) {
  const json = useMemo(() => JSON.stringify(t, null, 2), [t])
  return (
    <Section id="raw" title="Raw trace" note={`${(json.length / 1024).toFixed(1)} KB · exactly what the engine returned`}>
      <div className="inline raw__bar">
        <button type="button" className="act" onClick={() => navigator.clipboard?.writeText(json).then(() => toast({ title: 'Trace JSON copied' }), () => toast({ title: 'Copy blocked', tone: 'deny' }))}>Copy JSON</button>
      </div>
      <div className="tree"><Node k="trace" v={t} depth={0} /></div>
    </Section>
  )
}
function Node({ k, v, depth }: { k: string; v: unknown; depth: number }) {
  if (v !== null && typeof v === 'object') {
    const entries = Array.isArray(v) ? v.map((x, i) => [String(i), x] as const) : Object.entries(v as Record<string, unknown>)
    return (
      <details open={depth < 1} className="tree__node">
        <summary><span className="tree__k">{k}</span> <span className="mono-dim">{Array.isArray(v) ? `[${entries.length}]` : `{${entries.length}}`}</span></summary>
        <div className="tree__kids">{entries.map(([kk, vv]) => <Node key={kk} k={kk} v={vv} depth={depth + 1} />)}</div>
      </details>
    )
  }
  const cls = v === null ? 'null' : typeof v
  return (
    <div className="tree__leaf"><span className="tree__k">{k}</span>: {typeof v === 'string' && /^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(v) ? <Addr a={v} n={8} /> : <span className={`tree__v tree__v--${cls}`}>{JSON.stringify(v)}</span>}</div>
  )
}
