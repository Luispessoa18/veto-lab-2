import { useState } from 'react'
import { AnimatePresence, motion } from 'motion/react'
import { config, useConfig, versionId } from '../config/store'
import { diff, impact, type Change } from '../config/impact'
import { Enf, PageHead } from '../config/ui'
import { useEntries } from '../api/store'
import { DecisionBadge } from '../ui/Decision'
import HoldButton from '../ui/HoldButton'
import { toast } from '../ui/Toasts'
import { ago } from '../api/format'

export default function Deploy() {
  const s = useConfig()
  const entries = useEntries()
  const pub = s.history[0]
  const changes = diff(pub.config, s.draft)
  const imp = impact(entries, pub.config, s.draft)
  const [note, setNote] = useState('')
  const [compare, setCompare] = useState<string | null>(null)
  const groups = changes.reduce<Record<string, Change[]>>((g, c) => { (g[c.area] ??= []).push(c); return g }, {})
  const nextId = versionId(s.draft)
  const cmp = compare ? s.history.find((h) => h.id === compare) : null
  const cmpChanges = cmp ? diff(cmp.config, pub.config) : []

  const exportJson = () => {
    const blob = new Blob([JSON.stringify({ policyVersion: pub.id, publishedAt: new Date(pub.at).toISOString(), ...pub.config }, null, 2)], { type: 'application/json' })
    const a = document.createElement('a')
    a.href = URL.createObjectURL(blob); a.download = `aval-policy-${pub.id}.json`; a.click()
    setTimeout(() => URL.revokeObjectURL(a.href), 1000)
  }

  return (
    <div className="page">
      <PageHead kicker="deploy · policy versions" title="Review, publish, roll back"
        lede="The draft is everything edited since the last publish. Publishing makes it the active policy version; every verdict records which version judged it."
        right={<div className="verchip"><span className="mono-dim small">active</span><b>{pub.id}</b><span className="mono-dim small">{ago(pub.at)}</span></div>} />

      <div className="cfggrid">
        <div className="stack">
          <section className="panel">
            <div className="panel__head"><h2>Draft changes</h2><span className="mono-dim small">{pub.id} → {changes.length ? nextId : 'no changes'}</span></div>
            {!changes.length ? <p className="none">The draft matches the active version. Edit Policy, Manifests, Lists or Agents to stage a change.</p> : (
              <div className="diff">
                {Object.entries(groups).map(([area, list]) => (
                  <div key={area} className="diff__group">
                    <p className="diff__area">{area} <em>{list.length}</em></p>
                    <AnimatePresence initial={false}>
                      {list.map((c) => (
                        <motion.div key={c.label} layout className="diff__row" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
                          <span className="diff__label">{c.label}</span>
                          <span className="diff__from">{c.from}</span>
                          <span aria-hidden="true">→</span>
                          <span className="diff__to">{c.to}</span>
                          <Enf live={c.live} />
                        </motion.div>
                      ))}
                    </AnimatePresence>
                  </div>
                ))}
              </div>
            )}
          </section>

          <section className="panel">
            <div className="panel__head"><h2>Version history</h2><button type="button" className="act act--ghost" onClick={exportJson}>Export active as JSON</button></div>
            <ol className="timeline">
              {s.history.map((h, i) => (
                <li key={h.id + h.at} className={i === 0 ? 'is-active' : ''}>
                  <span className="timeline__dot" aria-hidden="true" />
                  <div className="timeline__body">
                    <div className="timeline__top"><b>{h.id}</b>{i === 0 && <span className="activechip">active</span>}<span className="mono-dim small">{ago(h.at)} · {h.by}</span></div>
                    <p className="small">{h.note || 'no note'}</p>
                    {i > 0 && (
                      <div className="inline">
                        <button type="button" className="act act--ghost" onClick={() => setCompare(compare === h.id ? null : h.id)}>{compare === h.id ? 'hide diff' : 'diff vs active'}</button>
                        <button type="button" className="act act--ghost" onClick={() => { config.rollback(h.id); toast({ title: `Draft reset to ${h.id}`, body: 'Publish to make it active again.' }) }}>restore to draft</button>
                      </div>
                    )}
                    {cmp && compare === h.id && (
                      <div className="diff diff--mini">
                        {cmpChanges.length ? cmpChanges.slice(0, 12).map((c) => (
                          <div key={c.label} className="diff__row"><span className="diff__label">{c.label}</span><span className="diff__from">{c.from}</span><span>→</span><span className="diff__to">{c.to}</span></div>
                        )) : <p className="none">Identical to the active version.</p>}
                      </div>
                    )}
                  </div>
                </li>
              ))}
            </ol>
          </section>
        </div>

        <aside className="stack sticky">
          <section className="panel impact">
            <div className="panel__head"><h2>Blast radius</h2><span className="mono-dim small">{imp.considered} real verdicts replayed</span></div>
            <div className="impact__nums">
              <div><b className="impact__n">{imp.flips.length}</b><span>verdicts change</span></div>
              <div><b className="impact__n">{imp.gate.length}</b><span>signing outcomes change</span></div>
              <div><b className="impact__n">{imp.manifest.length}</b><span>effect classifications change</span></div>
            </div>
            <ul className="flips">
              {imp.flips.slice(0, 5).map((f) => (
                <li key={f.entry.id}><code>{f.entry.trace.toolCall?.name}</code><span><DecisionBadge d={f.from} /> → <DecisionBadge d={f.to} /></span></li>
              ))}
              {imp.gate.slice(0, 4).map((g) => (
                <li key={'g' + g.entry.id}><code>{g.entry.trace.toolCall?.name}</code><span className="mono-dim small">{g.from} → <b className="ink">{g.to}</b></span></li>
              ))}
            </ul>
          </section>
          <section className="panel">
            <div className="panel__head"><h2>Publish</h2></div>
            <label className="fld"><span>What changed and why</span>
              <textarea id="publish-note" rows={3} value={note} placeholder="e.g. Hold flagged swaps for review during the hackathon demo" onChange={(e) => setNote(e.target.value)} />
            </label>
            <div className="publish">
              <HoldButton label={`Publish ${changes.length ? nextId : ''}`} disabled={!changes.length}
                onConfirm={() => { const v = config.publish(note.trim()); setNote(''); toast({ title: `Published ${v.id}`, body: `${changes.filter((c) => c.live).length} live setting(s) apply to the next evaluation.` }) }} />
              <button type="button" className="act act--ghost" disabled={!changes.length} onClick={() => config.discard()}>Discard draft</button>
            </div>
            <p className="mono-dim small">Live settings reach the engine on the next Scenario Lab run. Staged settings are versioned here for the engine's config API.</p>
          </section>
        </aside>
      </div>
    </div>
  )
}
