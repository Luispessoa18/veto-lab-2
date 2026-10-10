import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'motion/react'
import { TbCircleDotted } from 'react-icons/tb'
import { RxCross2 } from 'react-icons/rx'
import { LuPause } from 'react-icons/lu'
import type { Decision, Entry, Severity } from '../api/types'
import { EFFECT_GLYPH, EFFECT_LABEL, amount, named, short } from '../api/format'
import { config, useConfig } from '../config/store'
import { DecisionBadge, SevChip } from './Decision'
import Copy from './Copy'
import { toast } from './Toasts'
import { go } from '../router'
import { useReduced } from '../../lib/useReduced'

/*
 * The gate's interventions, inspectable in place. Left: a rail of refused or
 * held transactions (the FraudCard's motion language: dotted watcher ring, red
 * nodes, a beam running the line). Right: why the selected one was stopped,
 * read straight from its trace, with the actions an operator takes next.
 */
const SEV_RANK: Record<Severity, number> = { critical: 0, high: 1, medium: 2, low: 3 }
type Mode = 'deny' | 'flag' | 'all'

const when = (t: number) => {
  const d = new Date(t)
  return `${d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })} · ${d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })}`
}

export default function VetoInspector({ entries }: { entries: Entry[] }) {
  const reduced = useReduced()
  const cfg = useConfig()
  const [mode, setMode] = useState<Mode>('deny')
  const list = useMemo(
    () => entries.filter((e) => (mode === 'all' ? e.trace.decision.outcome !== 'allow' : e.trace.decision.outcome === mode)),
    [entries, mode],
  )
  const [sel, setSel] = useState<string | null>(null)
  const current = list.find((e) => e.id === sel) ?? list[0] ?? null
  useEffect(() => { if (current && current.id !== sel) setSel(current.id) }, [current, sel])

  // Beam target: the selected node's offset inside the rail.
  const railRef = useRef<HTMLOListElement>(null)
  const [beamY, setBeamY] = useState(0)
  useLayoutEffect(() => {
    const el = railRef.current?.querySelector<HTMLElement>(`[data-id="${current?.id}"]`)
    if (el) setBeamY(el.offsetTop + 14)
  }, [current?.id, list.length])

  const move = (d: number) => {
    if (!current) return
    const i = list.findIndex((e) => e.id === current.id)
    const next = list[Math.max(0, Math.min(list.length - 1, i + d))]
    if (next) { setSel(next.id); railRef.current?.querySelector<HTMLElement>(`[data-id="${next.id}"] button`)?.focus() }
  }

  const counts = { deny: entries.filter((e) => e.trace.decision.outcome === 'deny').length, flag: entries.filter((e) => e.trace.decision.outcome === 'flag').length }
  const last = list[0]

  return (
    <section className="vi panel" aria-labelledby="vi-title">
      <header className="vi__head">
        <div className="vi__watch">
          <motion.span className="vi__ring" animate={reduced ? undefined : { rotate: 360 }} transition={{ ease: 'linear', duration: 2.5, repeat: Infinity }}>
            <TbCircleDotted aria-hidden="true" />
          </motion.span>
          <div>
            <h2 id="vi-title">Gate interventions</h2>
            <p className="mono-dim small">
              {counts.deny} denied · {counts.flag} held this session{last ? ` · last ${when(last.at).split(' · ')[1]}` : ''}
            </p>
          </div>
        </div>
        <div className="seg" role="tablist" aria-label="Which interventions">
          {([['deny', `denied ${counts.deny}`], ['flag', `held ${counts.flag}`], ['all', 'all']] as [Mode, string][]).map(([m, label]) => (
            <button key={m} role="tab" aria-selected={mode === m} className={mode === m ? 'is-on' : ''} onClick={() => { setMode(m); setSel(null) }}>
              {mode === m && <motion.span layoutId="vi-mode" className="seg__hl" transition={{ type: 'spring', stiffness: 500, damping: 38 }} />}
              <span>{label}</span>
            </button>
          ))}
        </div>
      </header>

      {!list.length ? (
        <div className="vi__empty">
          <p>Nothing {mode === 'flag' ? 'held' : 'denied'} in this session yet.</p>
          <button type="button" className="act act--primary" onClick={() => go('evaluate')}>Run a scenario →</button>
        </div>
      ) : (
        <div className="vi__body">
          <ol
            ref={railRef}
            className="vi__rail"
            aria-label="Interventions, newest first"
            onKeyDown={(e) => {
              if (e.key === 'ArrowDown' || e.key === 'j') { e.preventDefault(); move(1) }
              if (e.key === 'ArrowUp' || e.key === 'k') { e.preventDefault(); move(-1) }
              if (e.key === 'Enter' && current) go(`overview/inspect/${current.id}`)
            }}
          >
            <span className="vi__line" aria-hidden="true" />
            {!reduced && <span className="vi__pulse" aria-hidden="true" />}
            <motion.span className="vi__beam" aria-hidden="true" animate={{ top: beamY }} transition={{ type: 'spring', stiffness: 260, damping: 28 }} />
            <AnimatePresence initial={false} mode="popLayout">
              {list.map((e, i) => {
                const d = e.trace.decision.outcome
                const on = current?.id === e.id
                const top = [...e.trace.findings].sort((a, b) => SEV_RANK[a.severity] - SEV_RANK[b.severity])[0]
                return (
                  <motion.li
                    key={e.id}
                    data-id={e.id}
                    layout
                    initial={{ opacity: 0, filter: 'blur(8px)', y: 6 }}
                    animate={{ opacity: 1, filter: 'blur(0px)', y: 0, transition: { delay: reduced ? 0 : Math.min(i, 8) * 0.06 } }}
                    exit={{ opacity: 0, filter: 'blur(6px)' }}
                    className={`vi__item ${on ? 'is-on' : ''}`}
                  >
                    <span className={`vi__node vi__node--${d}`} aria-hidden="true">{d === 'deny' ? <RxCross2 /> : <LuPause />}</span>
                    <button type="button" className="vi__btn" aria-current={on || undefined} onClick={() => setSel(e.id)}>
                      {on && <motion.span layoutId="vi-sel" className="vi__sel" transition={{ type: 'spring', stiffness: 500, damping: 40 }} />}
                      <span className="vi__tool">{e.trace.toolCall?.name ?? e.request.fixture}</span>
                      <span className="vi__why">{top ? top.rule : 'no findings'}</span>
                      <span className="vi__meta">{when(e.at)} · {short(e.trace.transaction.messageDigest, 4)}</span>
                    </button>
                  </motion.li>
                )
              })}
            </AnimatePresence>
          </ol>

          <AnimatePresence mode="wait">
            {current && <Detail key={current.id} e={current} denylisted={cfg.draft.lists.denylist.map((x) => x.value)} />}
          </AnimatePresence>
        </div>
      )}
      <p className="vi__keys mono-dim small" aria-hidden="true">↑ ↓ to move · enter to inspect</p>
    </section>
  )
}

function Detail({ e, denylisted }: { e: Entry; denylisted: string[] }) {
  const t = e.trace
  const d: Decision = t.decision.outcome
  const held = t.decision.steps.find((s) => s.held)
  const findings = [...t.findings].sort((a, b) => SEV_RANK[a.severity] - SEV_RANK[b.severity])
  const risky = t.counterparties.filter((c) => c.severity === 'critical' || c.severity === 'high')
  const m = t.toolCall?.manifest
  const notable = t.effects.filter((ef) => ef.type !== 'PROGRAM_INVOKE')

  const block = (addr: string) => {
    config.edit((c) => { if (!c.lists.denylist.some((x) => x.value === addr)) c.lists.denylist.unshift({ value: addr, note: `from ${t.toolCall?.name ?? 'trace'} ${short(t.transaction.messageDigest, 4)}`, addedAt: Date.now() }) })
    toast({ title: `Deny-list + ${short(addr, 6)}`, body: 'Staged in the draft. Publish it from Deploy.', tone: 'deny' })
  }

  return (
    <motion.article
      className={`vi__detail vi__detail--${d}`}
      initial={{ opacity: 0, x: 12, filter: 'blur(6px)' }}
      animate={{ opacity: 1, x: 0, filter: 'blur(0px)' }}
      exit={{ opacity: 0, x: -8, filter: 'blur(4px)', transition: { duration: 0.12 } }}
      transition={{ type: 'spring', stiffness: 320, damping: 32 }}
      aria-live="polite"
    >
      <div className="vi__dhead">
        <div>
          <p className="kicker">{e.source === 'live' ? 'live engine' : 'recorded'} · {e.request.provenance.replace('_', ' ')}</p>
          <h3 className="vi__dtitle">{t.toolCall?.name ?? e.request.fixture}</h3>
          <p className="mono-dim small">“{t.input.goal}”</p>
        </div>
        <DecisionBadge d={d} size="md" />
      </div>

      <div className="vi__why2">
        <span className="vi__lbl">stopped by</span>
        <span className="vi__rule">{held?.rule ?? '—'}</span>
        <span className="mono-dim small">{held?.detail}</span>
      </div>

      <div className="vi__cols">
        <div>
          <p className="vi__lbl">findings</p>
          <ul className="vi__list">
            {findings.map((f, i) => (
              <li key={i}><SevChip s={f.severity} /><code>{f.rule}</code></li>
            ))}
          </ul>
        </div>
        <div>
          <p className="vi__lbl">what it would have done</p>
          <ul className="vi__list">
            {notable.slice(0, 4).map((ef, i) => {
              const bad = m?.mustNot.includes(ef.type)
              return (
                <li key={i} className={bad ? 'is-bad' : ''}>
                  <span className="vi__glyph" aria-hidden="true">{EFFECT_GLYPH[ef.type]}</span>
                  <span>{EFFECT_LABEL[ef.type]}{amount(ef) ? <b> {amount(ef)}</b> : null}{ef.counterparty ? <> → {short(ef.counterparty)}</> : <span className="mono-dim"> {named(ef.account)}</span>}</span>
                </li>
              )
            })}
            {t.transaction.simulation !== 'executed' && <li className="is-bad">simulation {t.transaction.simulation}</li>}
          </ul>
        </div>
      </div>

      {risky.length > 0 && (
        <div className="vi__cps">
          <p className="vi__lbl">counterparty</p>
          {risky.map((c) => (
            <div key={c.address} className="vi__cp">
              <Copy value={c.address} display={short(c.address, 6)} />
              <span className={`origin origin--${c.origin}`}>{c.origin.replace('_', ' ')}</span>
              {denylisted.includes(c.address)
                ? <span className="mono-dim small">on deny-list (draft)</span>
                : <button type="button" className="act act--ghost" onClick={() => block(c.address)}>Block address</button>}
            </div>
          ))}
        </div>
      )}

      <div className="vi__foot">
        <span className="mono-dim small">digest <Copy value={t.transaction.messageDigest ?? ''} display={short(t.transaction.messageDigest, 6)} /> · slot {t.transaction.slot?.toLocaleString('en-US') ?? '—'} · {t.policyVersion}</span>
        <div className="inline">
          <button type="button" className="act" onClick={() => go(`evaluate/${e.request.fixture}`)}>Replay in lab</button>
          <button type="button" className="act act--primary" onClick={() => go(`overview/inspect/${e.id}`)}>Inspect →</button>
        </div>
      </div>
    </motion.article>
  )
}
