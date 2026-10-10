import { useRef, useState } from 'react'
import type { Entry } from '../api/types'
import { buckets } from '../api/issues'

/*
 * Verdicts over time as stacked bars (deny on the bottom, then flag, then
 * allow), above the action log. Drag across the chart to keep only that
 * window in the log, the way infrastructure dashboards filter an event list
 * from a spike; click without dragging or press "clear" to drop the window.
 */
const N = 48

export type Range = { from: number; to: number } | null

export default function FeedChart({ entries, range, onRange }: { entries: Entry[]; range: Range; onRange: (r: Range) => void }) {
  const box = useRef<HTMLDivElement>(null)
  const [drag, setDrag] = useState<{ a: number; b: number } | null>(null)
  const to = Date.now()
  const from = Math.min(to - 10 * 60_000, ...entries.map((e) => e.at))
  const span = to - from
  const b = buckets(entries, from, to, N)
  const max = Math.max(1, ...b.map((x) => x.allow + x.flag + x.deny))

  const frac = (clientX: number) => {
    const r = box.current!.getBoundingClientRect()
    return Math.max(0, Math.min(1, (clientX - r.left) / r.width))
  }
  const sel = drag ?? (range ? { a: (range.from - from) / span, b: (range.to - from) / span } : null)
  const lo = sel ? Math.min(sel.a, sel.b) : 0
  const hi = sel ? Math.max(sel.a, sel.b) : 0
  const clock = (t: number) => new Date(t).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })

  return (
    <section className="panel feedchart" aria-label="Verdicts over time">
      <div className="panel__head">
        <h2>Verdicts over time</h2>
        <span className="feedchart__hint">
          {range ? <>showing {clock(range.from)}–{clock(range.to)} · <button type="button" className="link" onClick={() => onRange(null)}>clear</button></> : 'drag across the chart to filter the log'}
        </span>
      </div>
      <div
        ref={box}
        className="feedchart__plot"
        role="slider"
        tabIndex={0}
        aria-label="Time window for the log"
        aria-valuetext={range ? `${clock(range.from)} to ${clock(range.to)}` : 'whole session'}
        onPointerDown={(e) => { e.currentTarget.setPointerCapture(e.pointerId); const f = frac(e.clientX); setDrag({ a: f, b: f }) }}
        onPointerMove={(e) => { if (drag) setDrag({ a: drag.a, b: frac(e.clientX) }) }}
        onPointerUp={() => {
          if (!drag) return
          const [x, y] = [Math.min(drag.a, drag.b), Math.max(drag.a, drag.b)]
          setDrag(null)
          onRange(y - x < 0.01 ? null : { from: from + x * span, to: from + y * span })
        }}
        onKeyDown={(e) => { if (e.key === 'Escape') onRange(null) }}
      >
        {b.map((x, i) => (
          <span key={i} className="feedchart__col">
            <i className="is-deny" style={{ height: `${(x.deny / max) * 100}%` }} />
            <i className="is-flag" style={{ height: `${(x.flag / max) * 100}%` }} />
            <i className="is-allow" style={{ height: `${(x.allow / max) * 100}%` }} />
          </span>
        ))}
        {sel && hi > lo && <span className="feedchart__sel" style={{ left: `${lo * 100}%`, width: `${(hi - lo) * 100}%` }} />}
      </div>
      <div className="feedchart__axis">
        <span>{clock(from)}</span>
        <span className="feedchart__legend"><i className="is-deny" />deny <i className="is-flag" />flag <i className="is-allow" />allow</span>
        <span>now</span>
      </div>
    </section>
  )
}
