import { useEffect, useMemo, useRef, useState } from 'react'
import Scramble from '../../components/Scramble'

/*
 * Runs: whole batch runs (thousands of cases) read from the lab API,
 * GET /admin/runs (list) and GET /admin/runs/<id> (one row per case). A run is
 * results/runs/<id>.jsonl, or the Solana benchmark's
 * results/solana_attack_simulation.jsonl. Everything here is computed in the
 * browser from those rows: score against the expected verdict, attacks
 * blocked / held / missed, false blocks, latency percentiles, and five views
 * (timeline, latency, attack types, stopped by, expected × got). Clicking an
 * attack type, a layer or a matrix cell filters the case list; dragging across
 * the timeline narrows it to that stretch of the run. A run file that is still
 * growing is re-read every ten seconds.
 */

type LabDecision = 'ALLOW' | 'BLOCK' | 'REVIEW' | 'ERROR' | string
interface Case { id?: string; at?: string; type?: string; expected?: LabDecision; decision: LabDecision; by?: string; ms?: number; error?: string }
interface RunMeta { id: string; file: string; bytes: number; modified: number; cases: number; source: 'run' | 'benchmark' }
interface Run { id: string; file: string; modified: number; rows: Case[]; skipped: number; truncated: boolean }
interface Filter { decision?: string; type?: string; by?: string; cell?: [string, string]; wrong?: boolean; range?: [number, number] }

const KIND: Record<string, 'allow' | 'flag' | 'deny'> = { ALLOW: 'allow', REVIEW: 'flag', BLOCK: 'deny' }
const kind = (d: LabDecision): 'allow' | 'flag' | 'deny' | 'err' => KIND[d] ?? 'err'
const WORD: Record<string, string> = { ALLOW: 'ALLOW', REVIEW: 'REVIEW', BLOCK: 'DENY', ERROR: 'ERROR', INVALID: 'INVALID' }
const word = (d: LabDecision) => WORD[d] ?? d
const ORDER = ['BLOCK', 'REVIEW', 'ALLOW', 'ERROR']
const VIEWS = [
  { k: 'timeline', label: 'timeline' },
  { k: 'latency', label: 'latency' },
  { k: 'types', label: 'attack types' },
  { k: 'layers', label: 'stopped by' },
  { k: 'matrix', label: 'expected × got' },
] as const
type View = (typeof VIEWS)[number]['k']
const PAGE = 100

const pct = (n: number, d: number) => (d ? `${((n / d) * 100).toFixed(1)}%` : '—')
const fmtMs = (v: number | null) => (v == null ? '—' : v >= 1000 ? `${(v / 1000).toFixed(2)} s` : `${v < 10 ? v.toFixed(1) : Math.round(v)} ms`)
const time = (t: number) => new Date(t).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })
const isAttack = (c: Case) => c.expected != null && c.expected !== 'ALLOW'
const scored = (c: Case) => c.expected != null
const wrong = (c: Case) => scored(c) && c.decision !== c.expected
function quantile(sorted: number[], q: number) {
  return sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))] : null
}
const fmtBytes = (n: number) => (n > 1e6 ? `${(n / 1e6).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1e3))} KB`)

export default function Runs({ runId }: { runId?: string }) {
  const [runs, setRuns] = useState<RunMeta[] | null>(null)
  const [state, setState] = useState<'checking' | 'up' | 'down'>('checking')
  const [run, setRun] = useState<Run | null>(null)
  const [loading, setLoading] = useState(false)
  const [view, setView] = useState<View>('timeline')
  const [f, setF] = useState<Filter>({})
  const [page, setPage] = useState(0)
  const loadedAt = useRef<{ id: string; modified: number } | null>(null)

  useEffect(() => {
    let alive = true
    const load = async () => {
      try {
        const r = await fetch('/lab/admin/runs', { cache: 'no-store', signal: AbortSignal.timeout(6000) })
        if (!r.ok) throw new Error(String(r.status))
        const d = (await r.json()) as { runs: RunMeta[] }
        if (alive) { setRuns(d.runs); setState('up') }
      } catch { if (alive) setState('down') }
    }
    load()
    const t = setInterval(load, 10_000)
    return () => { alive = false; clearInterval(t) }
  }, [])

  const selected = runId ?? runs?.[0]?.id
  const meta = runs?.find((r) => r.id === selected)

  /* Read the run when it is picked, and again whenever its file changes. */
  useEffect(() => {
    if (!selected || !meta) return
    const seen = loadedAt.current
    if (seen && seen.id === selected && seen.modified === meta.modified) return
    let alive = true
    const fresh = !seen || seen.id !== selected
    if (fresh) { setLoading(true); setRun(null); setF({}); setPage(0) }
    fetch(`/lab/admin/runs/${encodeURIComponent(selected)}`, { cache: 'no-store' })
      .then((r) => (r.ok ? (r.json() as Promise<Run>) : null))
      .then((d) => { if (alive && d) { setRun(d); loadedAt.current = { id: selected, modified: d.modified } } })
      .catch(() => {})
      .finally(() => { if (alive) setLoading(false) })
    return () => { alive = false }
  }, [selected, meta?.modified])

  const growing = meta ? Date.now() / 1000 - meta.modified < 60 : false

  return (
    <div className="page runs">
      <header className="page__head page__head--row">
        <div>
          <Scramble as="p" text="runs · batch results" className="kicker" />
          <h1>Runs</h1>
          <p className="issues__lede">Batch runs against the lab API, scored case by case against the expected verdict. Read from <code>results/runs/</code> and the Solana benchmark through <code>GET /admin/runs</code>.</p>
        </div>
        <span className={`live__src ${state === 'up' ? '' : 'is-snap'}`}><i aria-hidden="true" />{state === 'up' ? (growing ? 'run in progress · re-reading' : 'lab API · /lab') : state === 'down' ? 'lab API not running' : 'checking…'}</span>
      </header>

      {state === 'down' && <p className="callout">The lab API on port 8070 is not answering. Start it with <code>./iniciar_tudo.sh</code> (or <code>00_INICIAR_TUDO.bat</code>); runs are read from its <code>results/</code> folder.</p>}
      {state === 'up' && runs && !runs.length && <p className="callout">No runs yet. A run writes one line per case to <code>results/runs/&lt;run_id&gt;.jsonl</code> (request_id, attack_type, expected, decision, blocked_by, latency_ms, timestamp), or run <code>09_SIMULAR_ATAQUES_SOLANA</code>.</p>}

      {!!runs?.length && (
        <nav className="runs__pick" aria-label="Runs">
          {runs.map((r) => (
            <a key={r.id} href={`#/runs/${encodeURIComponent(r.id)}`} className={r.id === selected ? 'is-on' : ''} aria-current={r.id === selected ? 'page' : undefined}>
              <b>{r.id}</b>
              <small>{r.cases.toLocaleString()} cases · {fmtBytes(r.bytes)} · {new Date(r.modified * 1000).toLocaleString([], { dateStyle: 'short', timeStyle: 'short' })}{r.source === 'benchmark' ? ' · benchmark' : ''}</small>
            </a>
          ))}
        </nav>
      )}

      {loading && <p className="kicker">reading {meta?.cases.toLocaleString()} cases…</p>}
      {run && <RunView run={run} view={view} setView={setView} f={f} setF={(x) => { setF(x); setPage(0) }} page={page} setPage={setPage} />}
    </div>
  )
}

function RunView({ run, view, setView, f, setF, page, setPage }: { run: Run; view: View; setView: (v: View) => void; f: Filter; setF: (f: Filter) => void; page: number; setPage: (n: number) => void }) {
  const rows = run.rows
  const times = useMemo(() => rows.map((c) => (c.at ? Date.parse(c.at) : NaN)), [rows])

  const stats = useMemo(() => {
    const dec: Record<string, number> = {}
    let sc = 0, right = 0, attacks = 0, blocked = 0, held = 0, missed = 0, benign = 0, falseBlocks = 0
    for (const c of rows) {
      dec[c.decision] = (dec[c.decision] ?? 0) + 1
      if (!scored(c)) continue
      sc++
      if (c.decision === c.expected) right++
      if (isAttack(c)) {
        attacks++
        if (c.decision === 'BLOCK') blocked++
        else if (c.decision === 'REVIEW') held++
        else if (c.decision === 'ALLOW') missed++
      } else {
        benign++
        if (c.decision === 'BLOCK') falseBlocks++
      }
    }
    const ms = rows.map((c) => c.ms).filter((v): v is number => v != null).sort((a, b) => a - b)
    return { dec, sc, right, attacks, blocked, held, missed, benign, falseBlocks, p50: quantile(ms, 0.5), p95: quantile(ms, 0.95), p99: quantile(ms, 0.99) }
  }, [rows])

  const shown = useMemo(() => rows.filter((c, i) => {
    if (f.decision && c.decision !== f.decision) return false
    if (f.type && (c.type ?? '—') !== f.type) return false
    if (f.by && (c.by ?? '—') !== f.by) return false
    if (f.cell && ((c.expected ?? '—') !== f.cell[0] || c.decision !== f.cell[1])) return false
    if (f.wrong && !wrong(c)) return false
    if (f.range && !(times[i] >= f.range[0] && times[i] <= f.range[1])) return false
    return true
  }), [rows, times, f])

  const active = Object.entries(f).filter(([, v]) => v != null && v !== false)
  const chips: Record<string, string> = {
    decision: `verdict ${word(f.decision ?? '')}`, type: `type ${f.type}`, by: `stopped by ${f.by}`,
    cell: f.cell ? `expected ${word(f.cell[0])} · got ${word(f.cell[1])}` : '', wrong: 'only wrong',
    range: f.range ? `${time(f.range[0])}–${time(f.range[1])}` : '',
  }

  return (
    <>
      {(run.skipped > 0 || run.truncated) && <p className="callout small">{run.skipped > 0 && <>{run.skipped} unreadable line{run.skipped > 1 ? 's' : ''} skipped. </>}{run.truncated && <>Only the first {rows.length.toLocaleString()} cases are shown.</>}</p>}

      <section className="live__kpis runs__kpis" aria-label="Run summary">
        <div><span>cases</span><b>{rows.length.toLocaleString()}</b></div>
        <div><span>accuracy</span><b>{stats.sc ? pct(stats.right, stats.sc) : '—'}</b>{stats.sc < rows.length && <small>{stats.sc.toLocaleString()} scored</small>}</div>
        <div className="is-deny"><span>attacks blocked</span><b>{pct(stats.blocked, stats.attacks)}</b><small>{stats.blocked.toLocaleString()} of {stats.attacks.toLocaleString()} · {stats.held} held · {stats.missed} missed</small></div>
        <div className="is-flag"><span>false blocks</span><b>{pct(stats.falseBlocks, stats.benign)}</b><small>{stats.falseBlocks} of {stats.benign.toLocaleString()} benign</small></div>
        <div><span>latency p50 · p95 · p99</span><b className="runs__lat">{fmtMs(stats.p50)} <i>·</i> {fmtMs(stats.p95)} <i>·</i> {fmtMs(stats.p99)}</b><small>{(stats.dec.ERROR ?? 0).toLocaleString()} errors</small></div>
      </section>

      <section className="panel insights" aria-label="Run detail">
        <div className="insights__top">
          <div className="seg insights__tabs" role="tablist" aria-label="Chart">
            {VIEWS.map((v) => <button key={v.k} role="tab" aria-selected={view === v.k} className={view === v.k ? 'is-on' : ''} onClick={() => setView(v.k)}><span>{v.label}</span></button>)}
          </div>
          <div className="insights__period" role="group" aria-label="Verdict">
            {ORDER.filter((d) => stats.dec[d]).map((d) => (
              <button key={d} type="button" aria-pressed={f.decision === d} className={f.decision === d ? 'is-on' : ''} onClick={() => setF({ ...f, decision: f.decision === d ? undefined : d })}>{word(d)} {stats.dec[d].toLocaleString()}</button>
            ))}
            {stats.sc > 0 && <button type="button" aria-pressed={!!f.wrong} className={f.wrong ? 'is-on' : ''} onClick={() => setF({ ...f, wrong: !f.wrong || undefined })}>only wrong {(stats.sc - stats.right).toLocaleString()}</button>}
          </div>
        </div>
        {view === 'timeline' && <Timeline rows={rows} times={times} f={f} setF={setF} />}
        {view === 'latency' && <Latency rows={shown} />}
        {view === 'types' && <Types rows={rows} f={f} setF={setF} />}
        {view === 'layers' && <Layers rows={rows} f={f} setF={setF} />}
        {view === 'matrix' && <Matrix rows={rows} f={f} setF={setF} />}
      </section>

      <div className="runs__listhead">
        <h2>{shown.length === rows.length ? 'All cases' : `${shown.length.toLocaleString()} of ${rows.length.toLocaleString()} cases`}</h2>
        {active.length > 0 && (
          <span className="runs__chips">
            {active.map(([k]) => <button key={k} type="button" className="runs__chip" onClick={() => setF({ ...f, [k]: undefined })} aria-label={`Remove filter ${chips[k]}`}>{chips[k]} ×</button>)}
            <button type="button" className="link" onClick={() => setF({})}>clear</button>
          </span>
        )}
      </div>
      <div className="table runstable" role="table" aria-label="Cases">
        <div className="table__head" role="row"><span>verdict</span><span>case</span><span>attack type</span><span>expected</span><span>stopped by</span><span>latency</span><span>time</span></div>
        {shown.slice(page * PAGE, page * PAGE + PAGE).map((c, i) => {
          const k = kind(c.decision)
          const bad = wrong(c)
          return (
            <div key={`${c.id}-${i}`} role="row" className={`table__row runrow ${bad ? 'is-wrong' : ''}`} title={c.error ?? undefined}>
              <span><span className={`dbadge dbadge--${k === 'err' ? 'allow' : k} ${k === 'err' ? 'dbadge--err' : ''}`}>{word(c.decision)}</span></span>
              <span><code>{c.id ?? '—'}</code></span>
              <span className="mono-dim">{c.type ?? '—'}</span>
              <span className={bad ? 'runrow__miss' : 'mono-dim'}>{c.expected ? word(c.expected) : '—'}{bad && ' ✕'}</span>
              <span className="mono-dim">{c.by ?? (c.error ? 'error' : '—')}</span>
              <span className="mono-dim">{fmtMs(c.ms ?? null)}</span>
              <span className="mono-dim">{c.at ? time(Date.parse(c.at)) : '—'}</span>
            </div>
          )
        })}
        {!shown.length && <p className="empty">No cases match these filters.</p>}
      </div>
      {shown.length > PAGE && (
        <div className="runs__pager">
          <button type="button" className="link" disabled={page === 0} onClick={() => setPage(page - 1)}>← newer</button>
          <span className="mono-dim">{(page * PAGE + 1).toLocaleString()}–{Math.min(shown.length, (page + 1) * PAGE).toLocaleString()} of {shown.length.toLocaleString()}</span>
          <button type="button" className="link" disabled={(page + 1) * PAGE >= shown.length} onClick={() => setPage(page + 1)}>older →</button>
        </div>
      )}
    </>
  )
}

/* Cases over the run's wall-clock time, stacked by verdict; drag to narrow the list. */
function Timeline({ rows, times, f, setF }: { rows: Case[]; times: number[]; f: Filter; setF: (f: Filter) => void }) {
  const N = 60
  const box = useRef<HTMLDivElement>(null)
  const [drag, setDrag] = useState<[number, number] | null>(null)
  const ok = times.filter((t) => !Number.isNaN(t))
  if (ok.length < 2) return <p className="insights__empty">This run has no timestamps, so there is no timeline. The other views still apply.</p>
  let from = Infinity, to = -Infinity
  for (const t of ok) { if (t < from) from = t; if (t > to) to = t }
  const span = Math.max(1, to - from)
  const cols = Array.from({ length: N }, () => ({ deny: 0, flag: 0, allow: 0, err: 0 }))
  rows.forEach((c, i) => {
    if (Number.isNaN(times[i])) return
    cols[Math.min(N - 1, Math.floor(((times[i] - from) / span) * N))][kind(c.decision)]++
  })
  const max = Math.max(1, ...cols.map((x) => x.deny + x.flag + x.allow + x.err))
  const frac = (x: number) => { const r = box.current!.getBoundingClientRect(); return Math.max(0, Math.min(1, (x - r.left) / r.width)) }
  const sel = drag ? [Math.min(...drag), Math.max(...drag)] : f.range ? [(f.range[0] - from) / span, (f.range[1] - from) / span] : null
  const dur = span / 1000
  return (
    <figure className="insights__fig feedchart feedchart--bare">
      <div
        ref={box}
        className="feedchart__plot runs__plot"
        role="slider"
        tabIndex={0}
        aria-label="Stretch of the run"
        aria-valuetext={f.range ? `${time(f.range[0])} to ${time(f.range[1])}` : 'whole run'}
        onPointerDown={(e) => { e.currentTarget.setPointerCapture(e.pointerId); const v = frac(e.clientX); setDrag([v, v]) }}
        onPointerMove={(e) => { if (drag) setDrag([drag[0], frac(e.clientX)]) }}
        onPointerUp={() => {
          if (!drag) return
          const [a, b] = [Math.min(...drag), Math.max(...drag)]
          setDrag(null)
          setF({ ...f, range: b - a < 0.01 ? undefined : [from + a * span, from + b * span] })
        }}
        onKeyDown={(e) => { if (e.key === 'Escape') setF({ ...f, range: undefined }) }}
      >
        {cols.map((x, i) => (
          <span key={i} className="feedchart__col">
            <i className="is-err" style={{ height: `${(x.err / max) * 100}%` }} />
            <i className="is-deny" style={{ height: `${(x.deny / max) * 100}%` }} />
            <i className="is-flag" style={{ height: `${(x.flag / max) * 100}%` }} />
            <i className="is-allow" style={{ height: `${(x.allow / max) * 100}%` }} />
          </span>
        ))}
        {sel && sel[1] > sel[0] && <span className="feedchart__sel" style={{ left: `${sel[0] * 100}%`, width: `${(sel[1] - sel[0]) * 100}%` }} />}
      </div>
      <figcaption className="feedchart__axis">
        <span>{time(from)}</span>
        <span className="feedchart__legend"><i className="is-deny" />deny <i className="is-flag" />review <i className="is-allow" />allow <i className="is-err" />error · {dur >= 120 ? `${(dur / 60).toFixed(1)} min` : `${dur.toFixed(0)} s`} · {(ok.length / Math.max(1, dur)).toFixed(1)} cases/s · drag to narrow</span>
        <span>{time(to)}</span>
      </figcaption>
    </figure>
  )
}

/* How long cases took, on a log scale so 0.4 ms blacklist hits and 2 s simulations share one chart. */
function Latency({ rows }: { rows: Case[] }) {
  const ms = rows.map((c) => c.ms).filter((v): v is number => v != null && v > 0)
  if (!ms.length) return <p className="insights__empty">No latency recorded for these cases.</p>
  const edges = [0, 1, 3, 10, 30, 100, 300, 1000, 3000, 10000, Infinity]
  const label = (v: number) => (v >= 1000 ? `${v / 1000}s` : `${v}ms`)
  const bins = edges.slice(0, -1).map((lo, i) => ({ lo, hi: edges[i + 1], by: {} as Record<string, number>, n: 0 }))
  for (const c of rows) {
    if (c.ms == null || c.ms <= 0) continue
    const b = bins.find((x) => c.ms! >= x.lo && c.ms! < x.hi)!
    b.n++
    b.by[c.decision] = (b.by[c.decision] ?? 0) + 1
  }
  const max = Math.max(...bins.map((b) => b.n))
  return (
    <ul className="hbars runs__hist">
      {bins.map((b) => (
        <li key={b.lo}>
          <code>{b.hi === Infinity ? `≥ ${label(b.lo)}` : `${label(b.lo)} – ${label(b.hi)}`}</code>
          <span className="hbars__track hbars__track--stack" style={{ width: `${(b.n / max) * 100}%` }}>
            {ORDER.map((d) => b.by[d] > 0 && <i key={d} className={`is-${kind(d)}`} style={{ flexGrow: b.by[d] }} title={`${b.by[d]} ${word(d)}`} />)}
          </span>
          <b>{b.n.toLocaleString()}</b>
        </li>
      ))}
    </ul>
  )
}

/* Per attack type: what the gate did with it, and how often that matched the expected verdict. */
function Types({ rows, f, setF }: { rows: Case[]; f: Filter; setF: (f: Filter) => void }) {
  const m = new Map<string, { n: number; right: number; sc: number; expected?: string; by: Record<string, number> }>()
  for (const c of rows) {
    const k = c.type ?? '—'
    const cur = m.get(k) ?? { n: 0, right: 0, sc: 0, expected: c.expected, by: {} }
    cur.n++
    cur.by[c.decision] = (cur.by[c.decision] ?? 0) + 1
    if (scored(c)) { cur.sc++; if (c.decision === c.expected) cur.right++ }
    m.set(k, cur)
  }
  const list = [...m.entries()].sort((a, b) => a[1].right / Math.max(1, a[1].sc) - b[1].right / Math.max(1, b[1].sc) || b[1].n - a[1].n)
  const max = Math.max(...list.map(([, v]) => v.n))
  return (
    <ul className="hbars runs__types">
      {list.map(([k, v]) => (
        <li key={k}>
          <button type="button" className={`runs__rowbtn ${f.type === k ? 'is-on' : ''}`} aria-pressed={f.type === k} onClick={() => setF({ ...f, type: f.type === k ? undefined : k })}>
            <code>{k}</code><small>{v.expected ? `expects ${word(v.expected)}` : 'unscored'}</small>
          </button>
          <span className="hbars__track hbars__track--stack" style={{ width: `${(v.n / max) * 100}%` }}>
            {ORDER.map((d) => v.by[d] > 0 && <i key={d} className={`is-${kind(d)}`} style={{ flexGrow: v.by[d] }} title={`${v.by[d]} ${word(d)}`} />)}
          </span>
          <b className={v.sc && v.right < v.sc ? 'runs__acc is-low' : 'runs__acc'}>{v.sc ? pct(v.right, v.sc) : v.n.toLocaleString()}</b>
        </li>
      ))}
    </ul>
  )
}

/* Which layer stopped the cases that were stopped. */
function Layers({ rows, f, setF }: { rows: Case[]; f: Filter; setF: (f: Filter) => void }) {
  const m = new Map<string, Record<string, number>>()
  for (const c of rows) {
    if (c.decision === 'ALLOW') continue
    const k = c.by ?? (c.decision === 'ERROR' ? 'error' : '—')
    const cur = m.get(k) ?? {}
    cur[c.decision] = (cur[c.decision] ?? 0) + 1
    m.set(k, cur)
  }
  const list = [...m.entries()].map(([k, v]) => [k, v, Object.values(v).reduce((a, b) => a + b, 0)] as const).sort((a, b) => b[2] - a[2])
  if (!list.length) return <p className="insights__empty">Nothing was stopped in this run.</p>
  const max = list[0][2]
  return (
    <ul className="hbars">
      {list.map(([k, v, n]) => (
        <li key={k}>
          <button type="button" className={`runs__rowbtn ${f.by === k ? 'is-on' : ''}`} aria-pressed={f.by === k} onClick={() => setF({ ...f, by: f.by === k ? undefined : k })}><code>{k}</code></button>
          <span className="hbars__track hbars__track--stack" style={{ width: `${(n / max) * 100}%` }}>
            {ORDER.map((d) => v[d] > 0 && <i key={d} className={`is-${kind(d)}`} style={{ flexGrow: v[d] }} title={`${v[d]} ${word(d)}`} />)}
          </span>
          <b>{n.toLocaleString()}</b>
        </li>
      ))}
    </ul>
  )
}

/* Expected verdict (rows) against the verdict the gate gave (columns). */
function Matrix({ rows, f, setF }: { rows: Case[]; f: Filter; setF: (f: Filter) => void }) {
  const exp = ORDER.filter((d) => rows.some((c) => c.expected === d))
  if (!exp.length) return <p className="insights__empty">This run has no expected verdicts, so it cannot be scored.</p>
  const got = ORDER.filter((d) => rows.some((c) => c.decision === d))
  const n = (e: string, g: string) => rows.reduce((a, c) => a + (c.expected === e && c.decision === g ? 1 : 0), 0)
  return (
    <div className="runs__matrixwrap">
    <table className="runs__matrix">
      <thead><tr><th scope="col">expected ↓ · got →</th>{got.map((g) => <th key={g} scope="col">{word(g)}</th>)}</tr></thead>
      <tbody>
        {exp.map((e) => {
          const total = rows.reduce((a, c) => a + (c.expected === e ? 1 : 0), 0)
          return (
            <tr key={e}>
              <th scope="row">{word(e)}</th>
              {got.map((g) => {
                const v = n(e, g)
                const on = f.cell?.[0] === e && f.cell?.[1] === g
                return (
                  <td key={g} className={e === g ? 'is-right' : v ? 'is-wrong' : ''}>
                    <button type="button" disabled={!v} aria-pressed={on} className={on ? 'is-on' : ''} onClick={() => setF({ ...f, cell: on ? undefined : [e, g] })}>
                      <b>{v.toLocaleString()}</b><small>{pct(v, total)}</small>
                    </button>
                  </td>
                )
              })}
            </tr>
          )
        })}
      </tbody>
    </table>
    </div>
  )
}
