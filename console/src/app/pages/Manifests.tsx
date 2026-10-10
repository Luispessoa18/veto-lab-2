import { useState } from 'react'
import { motion } from 'motion/react'
import { config, useConfig, type EffectClass, type ManifestCfg, type Pattern, type Scope } from '../config/store'
import { EFFECTS, classOf, impact } from '../config/impact'
import { Choice, Enf, PageHead, Switch } from '../config/ui'
import { EFFECT_GLYPH, EFFECT_LABEL } from '../api/format'
import { useEntries } from '../api/store'
import type { EffectType } from '../api/types'

const CLASSES: { v: EffectClass; label: string; hint: string }[] = [
  { v: 'must', label: 'must', hint: 'has to happen' },
  { v: 'may', label: 'may', hint: 'allowed' },
  { v: 'mustNot', label: 'must not', hint: 'presence = immediate violation' },
  { v: 'none', label: '—', hint: 'not declared: raises undeclared.*' },
]
const SCOPES: { v: Scope; label: string }[] = [
  { v: 'args_only', label: 'args only' },
  { v: 'args_plus_derived', label: 'args + derived' },
  { v: 'open', label: 'open' },
]

/* Move an effect to another class. Constraints written in the published
   manifest (owner, mint, maxAmount, tolerance) come back if it returns. */
function setClass(m: ManifestCfg, t: EffectType, to: EffectClass, published?: ManifestCfg) {
  m.must = m.must.filter((p) => p.type !== t)
  m.may = m.may.filter((p) => p.type !== t)
  m.mustNot = m.mustNot.filter((x) => x !== t)
  const orig = (k: 'must' | 'may') => published?.[k].filter((p) => p.type === t) ?? []
  if (to === 'must') m.must.push(...(orig('must').length ? orig('must') : [{ type: t }]))
  if (to === 'may') m.may.push(...(orig('may').length ? orig('may') : [{ type: t }]))
  if (to === 'mustNot') m.mustNot.push(t)
}

function constraints(p: Pattern) {
  return Object.entries(p).filter(([k]) => k !== 'type').map(([k, v]) => `${k} ${v}`)
}

export default function Manifests() {
  const s = useConfig()
  if (!s.draft.manifests.length) {
    return (
      <div className="page">
        <header className="page__head">
          <p className="kicker">manifests · per tool</p>
          <h1>Tool manifests</h1>
        </header>
        <p className="callout">No manifests in this copy. A manifest says what a tool must, may and must never do; the engine ships them. Add them to the console's config to edit them here.</p>
      </div>
    )
  }
  return <ManifestEditor />
}

function ManifestEditor() {
  const s = useConfig()
  const entries = useEntries()
  const [sel, setSel] = useState(s.draft.manifests[0].tool)
  const [q, setQ] = useState('')
  const m = s.draft.manifests.find((x) => x.tool === sel)!
  const pm = s.history[0].config.manifests.find((x) => x.tool === sel)
  const changed = (tool: string) => JSON.stringify(s.draft.manifests.find((x) => x.tool === tool)) !== JSON.stringify(s.history[0].config.manifests.find((x) => x.tool === tool))
  const imp = impact(entries.filter((e) => e.trace.toolCall?.name === sel), s.history[0].config, s.draft)
  const seen = entries.filter((e) => e.trace.toolCall?.name === sel).length
  const edit = (r: (x: ManifestCfg) => void) => config.edit((c) => { r(c.manifests.find((x) => x.tool === sel)!) })

  return (
    <div className="page">
      <PageHead
        kicker="configure · tool manifests"
        title="What each tool is allowed to do"
        lede="A manifest is the promise a tool makes. The engine compares the simulated effects of every transaction against it: a missing must, an undeclared effect or any must-not becomes a finding."
        right={<Enf />}
      />
      <div className="mfgrid">
        <aside className="panel mflist">
          <input className="search" placeholder="Filter tools" aria-label="Filter tools" value={q} onChange={(e) => setQ(e.target.value)} />
          <ul>
            {s.draft.manifests.filter((x) => x.tool.includes(q.toLowerCase())).map((x) => (
              <li key={x.tool}>
                <button type="button" className={x.tool === sel ? 'is-on' : ''} onClick={() => setSel(x.tool)}>
                  {x.tool === sel && <motion.span layoutId="mf-hl" className="mflist__hl" transition={{ type: 'spring', stiffness: 520, damping: 40 }} />}
                  <span className={`mflist__dot ${x.enabled ? '' : 'is-off'}`} aria-hidden="true" />
                  <span className="mflist__name">{x.tool}</span>
                  <span className="mflist__v">v{x.version}{changed(x.tool) && <em className="dirty"> ●</em>}</span>
                </button>
              </li>
            ))}
          </ul>
        </aside>

        <section className="panel mfedit">
          <div className="mfedit__head">
            <div>
              <h2 className="mfedit__title">{m.tool}</h2>
              <p className="mono-dim small">
                version {m.version}{changed(sel) && <> → <b className="ink">{Number(m.version) + 1}</b> on publish</>} · args {Object.entries(m.args).map(([k, v]) => `${k}:${v}`).join(', ')}
              </p>
            </div>
            <label className="inline">
              <span className="mono-dim small">{m.enabled ? 'enforced' : 'disabled: tool runs unchecked'}</span>
              <Switch label={`Enforce ${m.tool} manifest`} on={m.enabled} onChange={(v) => edit((x) => { x.enabled = v })} />
            </label>
          </div>

          <div className="mfscope">
            <span className="cfgrow__label">Account scope</span>
            <Choice id={`scope-${sel}`} value={m.accountScope} options={SCOPES} onChange={(v) => edit((x) => { x.accountScope = v })} />
            <span className="cfgrow__hint">Accounts touched outside the scope are a high-severity finding.</span>
          </div>

          <div className="matrix" role="table" aria-label={`${m.tool} effects`}>
            <div className="matrix__head" role="row">
              <span role="columnheader">effect</span>
              {CLASSES.map((c) => <span key={c.v} role="columnheader" title={c.hint}>{c.label}</span>)}
            </div>
            {EFFECTS.map((t) => {
              const cur = classOf(m, t)
              const was = pm ? classOf(pm, t) : cur
              const pats = [...m.must, ...m.may].filter((p) => p.type === t).flatMap(constraints)
              return (
                <div key={t} role="row" className={`matrix__row ${cur !== was ? 'is-changed' : ''} cls--${cur}`}>
                  <span className="matrix__eff" role="rowheader">
                    <i aria-hidden="true">{EFFECT_GLYPH[t]}</i>
                    <span>{EFFECT_LABEL[t]}<small>{t}</small></span>
                    {!!pats.length && <span className="matrix__cons">{pats.map((c) => <code key={c}>{c}</code>)}</span>}
                  </span>
                  {CLASSES.map((c) => (
                    <span key={c.v} role="cell" className="matrix__cell">
                      <button
                        type="button"
                        aria-pressed={cur === c.v}
                        aria-label={`${EFFECT_LABEL[t]}: ${c.label}`}
                        className={`cellbtn cellbtn--${c.v} ${cur === c.v ? 'is-on' : ''}`}
                        onClick={() => edit((x) => setClass(x, t, c.v, pm))}
                      >
                        {cur === c.v && <motion.span layoutId={`cell-${sel}-${t}`} className="cellbtn__fill" transition={{ type: 'spring', stiffness: 600, damping: 38 }} />}
                        <span>{cur === c.v ? (c.v === 'mustNot' ? '✕' : c.v === 'none' ? '·' : '■') : ''}</span>
                      </button>
                    </span>
                  ))}
                </div>
              )
            })}
          </div>

          <div className="mfimpact">
            <span className="cfgrow__label">Against this session</span>
            <span className="mono-dim small">
              {seen ? `${seen} recorded ${sel} action${seen === 1 ? '' : 's'}` : `no recorded ${sel} actions yet; run one from the Scenario Lab`}
              {imp.manifest.length > 0 && ` · ${imp.manifest.length} effect classifications change`}
            </span>
            {imp.manifest.slice(0, 4).map((x, i) => (
              <span key={i} className="mfimpact__row"><code>{x.effect}</code> {x.from} → <b className={x.to === 'mustNot' ? 'deny' : ''}>{x.to}</b></span>
            ))}
          </div>
        </section>
      </div>
    </div>
  )
}
