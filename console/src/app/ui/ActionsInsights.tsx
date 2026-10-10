import { useMemo, useRef, useState } from 'react'
import type { Decision, Entry } from '../api/types'
import { agentOf, primary } from '../api/issues'
import FeedChart, { type Range } from './FeedChart'

/*
 * A detailed look at one period of the gate's work, above the action log.
 * Pick a period (or drag across a time chart to narrow it); every view and
 * the log below follow the same window:
 *   volume   verdicts per time slice, stacked by decision
 *   latency  engine time per slice (median and slowest), live verdicts only
 *   mix      share of allow / flag / deny
 *   rules    which rules weighed most, by count
 *   agents   verdicts per agent, stacked by decision
 */

export type View = 'volume' | 'latency' | 'mix' | 'rules' | 'agents'
const VIEWS: { k: View; label: string }[] = [
  { k: 'volume', label: 'volume' },
  { k: 'latency', label: 'latency' },
  { k: 'mix', label: 'decision mix' },
  { k: 'rules', label: 'top rules' },
  { k: 'agents', label: 'by agent' },
]
const PERIODS = [
  { k: '15m', label: 'last 15 min', ms: 15 * 60_000 },
  { k: '1h', label: 'last hour', ms: 60 * 60_000 },
  { k: 'all', label: 'session', ms: Infinity },
] as const
const D: Decision[] = ['deny', 'flag', 'allow']

const clock = (t: number) => new Date(t).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
const pct = (n: number, d: number) => (d ? `${Math.round((n / d) * 100)}%` : '—')
function quantile(xs: number[], q: number) {
  if (!xs.length) return null
  const s = [...xs].sort((a, b) => a - b)
  return s[Math.min(s.length - 1, Math.floor(q * s.length))]
}

/** The window every view and the log use: a dragged range wins over the period. */
export function useWindow(entries: Entry[]) {
  const [period, setPeriod] = useState<(typeof PERIODS)[number]['k']>('all')
  const [range, setRange] = useState<Range>(null)
  const win = useMemo(() => {
    if (range) return range
    const to = Date.now()
    const p = PERIODS.find((x) => x.k === period)!
    const first = entries.length ? Math.min(...entries.map((e) => e.at)) : to - 10 * 60_000
    return { from: p.ms === Infinity ? Math.min(first, to - 10 * 60_000) : to - p.ms, to }
  }, [entries, period, range])
  const inWin = entries.filter((e) => e.at >= win.from && e.at <= win.to)
  return { period, setPeriod, range, setRange, win, inWin }
}

export default function ActionsInsights({ entries, w }: { entries: Entry[]; w: ReturnType<typeof useWindow> }) {
  const [view, setView] = useState<View>('volume')
  const { inWin, win } = w
  const by = { allow: 0, flag: 0, deny: 0 }
  inWin.forEach((e) => by[e.trace.decision.outcome]++)
  const live = inWin.filter((e) => e.source === 'live' && e.ms > 0).map((e) => e.ms)
  const p50 = quantile(live, 0.5), p95 = quantile(live, 0.95)

  return (
    <section className="panel insights" aria-label="Detailed view of the period">
      <div className="insights__top">
        <div className="seg insights__tabs" role="tablist" aria-label="Chart">
          {VIEWS.map((v) => (
            <button key={v.k} role="tab" aria-selected={view === v.k} className={view === v.k ? 'is-on' : ''} onClick={() => setView(v.k)}>
              <span>{v.label}</span>
            </button>
          ))}
        </div>
        <div className="insights__period" role="group" aria-label="Period">
          {PERIODS.map((p) => (
            <button key={p.k} type="button" aria-pressed={!w.range && w.period === p.k} className={!w.range && w.period === p.k ? 'is-on' : ''} onClick={() => { w.setRange(null); w.setPeriod(p.k) }}>{p.label}</button>
          ))}
          {w.range && <span className="insights__custom">{clock(w.range.from)}–{clock(w.range.to)} <button type="button" className="link" onClick={() => w.setRange(null)}>clear</button></span>}
        </div>
      </div>

      <dl className="insights__sum">
        <div><dt>verdicts</dt><dd>{inWin.length}</dd></div>
        <div><dt>denied</dt><dd className="is-deny">{by.deny} <small>{pct(by.deny, inWin.length)}</small></dd></div>
        <div><dt>held</dt><dd className="is-flag">{by.flag} <small>{pct(by.flag, inWin.length)}</small></dd></div>
        <div><dt>allowed</dt><dd>{by.allow} <small>{pct(by.allow, inWin.length)}</small></dd></div>
        <div><dt>engine p50 · p95</dt><dd>{p50 == null ? '—' : `${p50} · ${p95} ms`}</dd></div>
      </dl>

      {view === 'volume' && <FeedChart entries={entries} range={w.range} onRange={w.setRange} window={win} bare />}
      {view === 'latency' && <Latency entries={entries.filter((e) => e.source === 'live' && e.ms > 0)} win={win} onRange={w.setRange} />}
      {view === 'mix' && <Mix by={by} total={inWin.length} />}
      {view === 'rules' && <Rules entries={inWin} />}
      {view === 'agents' && <Agents entries={inWin} />}
    </section>
  )
}

/* Engine latency per time slice: a band from median to slowest, median as a line. */
function Latency({ entries, win, onRange }: { entries: Entry[]; win: { from: number; to: number }; onRange: (r: Range) => void }) {
  const N = 36, W = 600, H = 110
  const span = Math.max(1, win.to - win.from)
  const slots = Array.from({ length: N }, () => [] as number[])
  for (const e of entries) {
    if (e.at < win.from || e.at > win.to) continue
    slots[Math.min(N - 1, Math.floor(((e.at - win.from) / span) * N))].push(e.ms)
  }
  const med = slots.map((s) => quantile(s, 0.5)), max = slots.map((s) => (s.length ? Math.max(...s) : null))
  const top = Math.max(1, ...max.map((v) => v ?? 0))
  const x = (i: number) => (i + 0.5) * (W / N), y = (v: number) => H - 6 - (v / top) * (H - 16)
  const pts = (arr: (number | null)[]) => arr.map((v, i) => (v == null ? null : `${x(i)},${y(v)}`)).filter(Boolean).join(' ')
  const drag = useDrag(win, onRange)
  if (!entries.some((e) => e.at >= win.from && e.at <= win.to)) return <p className="insights__empty">No live verdicts in this period. Recorded ones have no engine time.</p>
  return (
    <figure className="insights__fig">
      <svg viewBox={`0 0 ${W} ${H}`} className="lat" preserveAspectRatio="none" {...drag.handlers} role="img" aria-label={`Engine latency, slowest ${top} ms`}>
        {[0.5, 1].map((g) => <line key={g} x1="0" x2={W} y1={y(top * g)} y2={y(top * g)} className="lat__grid" />)}
        {max.map((v, i) => v != null && med[i] != null && <line key={i} x1={x(i)} x2={x(i)} y1={y(v)} y2={y(med[i]!)} className="lat__band" />)}
        <polyline points={pts(med)} className="lat__line" />
        {med.map((v, i) => v != null && <circle key={i} cx={x(i)} cy={y(v)} r="2.2" className="lat__dot" />)}
        {drag.sel && <rect x={drag.sel[0] * W} width={(drag.sel[1] - drag.sel[0]) * W} y="0" height={H} className="lat__sel" />}
      </svg>
      <figcaption className="feedchart__axis"><span>{clock(win.from)}</span><span className="feedchart__legend"><i className="lat__k lat__k--line" />median <i className="lat__k lat__k--band" />up to slowest · top {top} ms</span><span>{clock(win.to)}</span></figcaption>
    </figure>
  )
}

function useDrag(win: { from: number; to: number }, onRange: (r: Range) => void) {
  const [d, setD] = useState<[number, number] | null>(null)
  const el = useRef<Element | null>(null)
  const f = (cx: number) => { const r = el.current!.getBoundingClientRect(); return Math.max(0, Math.min(1, (cx - r.left) / r.width)) }
  const span = win.to - win.from
  return {
    sel: d ? [Math.min(...d), Math.max(...d)] as [number, number] : null,
    handlers: {
      onPointerDown: (e: React.PointerEvent) => { el.current = e.currentTarget; (e.currentTarget as Element).setPointerCapture(e.pointerId); const v = f(e.clientX); setD([v, v]) },
      onPointerMove: (e: React.PointerEvent) => { if (d) setD([d[0], f(e.clientX)]) },
      onPointerUp: () => {
        if (!d) return
        const [a, b] = [Math.min(...d), Math.max(...d)]
        setD(null)
        if (b - a > 0.01) onRange({ from: win.from + a * span, to: win.from + b * span })
      },
      style: { cursor: 'crosshair', touchAction: 'none' } as React.CSSProperties,
    },
  }
}

/* Share of each decision, as one proportional bar with a legend. */
function Mix({ by, total }: { by: Record<Decision, number>; total: number }) {
  if (!total) return <p className="insights__empty">No verdicts in this period.</p>
  return (
    <figure className="insights__fig">
      <div className="mix" role="img" aria-label={`${by.deny} denied, ${by.flag} held, ${by.allow} allowed`}>
        {D.map((d) => by[d] > 0 && <span key={d} className={`mix__seg is-${d}`} style={{ flexGrow: by[d] }}><b>{pct(by[d], total)}</b></span>)}
      </div>
      <ul className="mix__legend">
        {D.map((d) => <li key={d}><i className={`is-${d}`} />{d === 'deny' ? 'denied' : d === 'flag' ? 'held for a person' : 'allowed'}<b>{by[d]}</b></li>)}
      </ul>
    </figure>
  )
}

/* The rule that weighed most on each flagged or denied verdict, counted. */
function Rules({ entries }: { entries: Entry[] }) {
  const m = new Map<string, { n: number; d: Decision }>()
  for (const e of entries) {
    if (e.trace.decision.outcome === 'allow') continue
    const r = primary(e)?.rule ?? 'decision.held'
    const cur = m.get(r) ?? { n: 0, d: e.trace.decision.outcome }
    cur.n++
    if (e.trace.decision.outcome === 'deny') cur.d = 'deny'
    m.set(r, cur)
  }
  const rows = [...m.entries()].sort((a, b) => b[1].n - a[1].n).slice(0, 8)
  if (!rows.length) return <p className="insights__empty">Nothing flagged or denied in this period.</p>
  const max = rows[0][1].n
  return (
    <ul className="hbars">
      {rows.map(([r, v]) => (
        <li key={r}><code>{r}</code><span className="hbars__track"><i className={`is-${v.d}`} style={{ width: `${(v.n / max) * 100}%` }} /></span><b>{v.n}</b></li>
      ))}
    </ul>
  )
}

/* Verdicts per agent, stacked by decision. */
function Agents({ entries }: { entries: Entry[] }) {
  const m = new Map<string, Record<Decision, number>>()
  for (const e of entries) {
    const a = agentOf(e)
    const cur = m.get(a) ?? { allow: 0, flag: 0, deny: 0 }
    cur[e.trace.decision.outcome]++
    m.set(a, cur)
  }
  const rows = [...m.entries()].sort((a, b) => (b[1].deny - a[1].deny) || (b[1].allow + b[1].flag + b[1].deny) - (a[1].allow + a[1].flag + a[1].deny))
  if (!rows.length) return <p className="insights__empty">No verdicts in this period.</p>
  const max = Math.max(...rows.map(([, v]) => v.allow + v.flag + v.deny))
  return (
    <ul className="hbars">
      {rows.map(([a, v]) => {
        const n = v.allow + v.flag + v.deny
        return (
          <li key={a}>
            <code>{a}</code>
            <span className="hbars__track hbars__track--stack" style={{ width: `${(n / max) * 100}%` }}>
              {D.map((d) => v[d] > 0 && <i key={d} className={`is-${d}`} style={{ flexGrow: v[d] }} title={`${v[d]} ${d}`} />)}
            </span>
            <b>{n}</b>
          </li>
        )
      })}
    </ul>
  )
}

