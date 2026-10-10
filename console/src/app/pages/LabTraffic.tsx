import { Fragment, useEffect, useState } from 'react'
import Scramble from '../../components/Scramble'

/*
 * Lab traffic: requests this repository's own API (src/unified_api.py, port
 * 8070, reached through /lab) has judged, read from its audit log via
 * GET /admin/requests every two seconds. Each request walks the lab's layers
 * (blacklist → Prompt Guard → AI → simulation); the console shows the
 * decision, which layer stopped it and the path it took. When the Solana
 * attack benchmark has been run (09_SIMULAR_ATAQUES_SOLANA), its summary from
 * GET /admin/report sits on top.
 */

type LabDecision = 'ALLOW' | 'BLOCK' | 'REVIEW' | 'ERROR' | 'INVALID' | string
interface Layer { layer: string; decision: LabDecision; reason?: string; message?: string; security_alert?: boolean; prompt_guard?: { malicious_score?: number; threshold?: number } }
interface Row { timestamp: string; route: string; request_id: string; decision: LabDecision; blocked?: boolean; blocked_by?: string | null; layers?: Layer[]; latency_ms?: number; error?: string }
interface Requests { summary: { total: number; blocked: number; average_latency_ms: number; max_latency_ms: number; decisions: Record<string, number> }; requests: Row[] }
interface Report { cases?: number; accuracy?: number | null; attack_detection?: { attacks?: number; blocked?: number; missed?: string[] }; funnel?: Record<string, number>; generated_at?: string }

/* The lab's words mapped onto the console's verdict colours. */
const KIND: Record<string, 'allow' | 'flag' | 'deny' | 'err'> = { ALLOW: 'allow', REVIEW: 'flag', BLOCK: 'deny' }
const kind = (d: LabDecision) => KIND[d] ?? 'err'
const WORD: Record<string, string> = { ALLOW: 'ALLOW', REVIEW: 'REVIEW', BLOCK: 'DENY', ERROR: 'ERROR', INVALID: 'INVALID' }

const time = (iso: string) => new Date(iso).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })

export default function LabTraffic() {
  const [data, setData] = useState<Requests | null>(null)
  const [report, setReport] = useState<Report | null>(null)
  const [state, setState] = useState<'checking' | 'up' | 'down'>('checking')
  const [open, setOpen] = useState<string | null>(null)

  useEffect(() => {
    let alive = true
    const load = async () => {
      try {
        const r = await fetch('/lab/admin/requests?limit=200', { cache: 'no-store', signal: AbortSignal.timeout(4000) })
        if (!r.ok) throw new Error(String(r.status))
        const d = (await r.json()) as Requests
        if (alive) { setData(d); setState('up') }
      } catch { if (alive) setState('down') }
    }
    const loadReport = async () => {
      try {
        const r = await fetch('/lab/admin/report', { cache: 'no-store', signal: AbortSignal.timeout(4000) })
        if (alive) setReport(r.ok ? ((await r.json()) as Report) : null)
      } catch { if (alive) setReport(null) }
    }
    load(); loadReport()
    const t = setInterval(load, 2000)
    const t2 = setInterval(loadReport, 30_000)
    return () => { alive = false; clearInterval(t); clearInterval(t2) }
  }, [])

  const rows = data?.requests ?? []
  const s = data?.summary

  return (
    <div className="page lab">
      <header className="page__head page__head--row">
        <div>
          <Scramble as="p" text="lab traffic · this repository's API" className="kicker" />
          <h1>Lab traffic</h1>
          <p className="issues__lede">Requests judged by the lab API (<code>src/unified_api.py</code>): blacklist, Prompt Guard, AI and simulation, in that order. Refreshed every two seconds from its audit log.</p>
        </div>
        <span className={`live__src ${state === 'up' ? '' : 'is-snap'}`}><i aria-hidden="true" />{state === 'up' ? 'lab API live · /lab' : state === 'down' ? 'lab API not running' : 'checking…'}</span>
      </header>

      {state === 'down' && (
        <p className="callout">The lab API on port 8070 is not answering. Start it with <code>./iniciar_tudo.sh</code> (or <code>00_INICIAR_TUDO.bat</code>), then send requests to <code>/v1/transactions/verify</code> or run <code>09_SIMULAR_ATAQUES_SOLANA</code>.</p>
      )}

      {report && (
        <section className="panel labbench" aria-label="Solana attack benchmark">
          <div className="panel__head"><h2>Solana attack benchmark</h2><span className="mono-dim">{report.generated_at ? `generated ${new Date(report.generated_at).toLocaleString()}` : 'last run'}</span></div>
          <dl className="insights__sum">
            <div><dt>cases</dt><dd>{report.cases ?? '—'}</dd></div>
            <div><dt>pipeline accuracy</dt><dd>{report.accuracy == null ? '—' : `${(report.accuracy * 100).toFixed(1)}%`}</dd></div>
            <div><dt>attacks blocked</dt><dd className="is-deny">{report.attack_detection?.blocked ?? 0} <small>of {report.attack_detection?.attacks ?? 0}</small></dd></div>
            <div><dt>missed</dt><dd>{report.attack_detection?.missed?.length ?? 0}</dd></div>
            <div><dt>reached simulation</dt><dd>{report.funnel?.reached_simulation ?? '—'}</dd></div>
          </dl>
        </section>
      )}

      {s && (
        <section className="live__kpis" aria-label="Recent requests">
          <div><span>requests</span><b>{s.total}</b></div>
          <div className="is-deny"><span>blocked</span><b>{s.blocked}</b></div>
          <div className="is-flag"><span>review</span><b>{s.decisions.REVIEW ?? 0}</b></div>
          <div><span>avg latency</span><b>{s.average_latency_ms} ms</b></div>
          <div><span>slowest</span><b>{s.max_latency_ms} ms</b></div>
        </section>
      )}

      <div className="table labtable" role="table" aria-label="Lab requests">
        <div className="table__head" role="row">
          <span>verdict</span><span>request</span><span>stopped by</span><span>layers</span><span>latency</span><span>time</span>
        </div>
        {rows.map((r) => {
          const id = r.request_id + r.timestamp
          const k = kind(r.decision)
          return (
            <Fragment key={id}>
              <button type="button" role="row" className={`table__row labrow labrow--${k}`} aria-expanded={open === id} onClick={() => setOpen(open === id ? null : id)}>
                <span><span className={`dbadge dbadge--${k === 'err' ? 'allow' : k} ${k === 'err' ? 'dbadge--err' : ''}`}>{WORD[r.decision] ?? r.decision}</span></span>
                <span className="labrow__id"><code>{r.request_id}</code><small>{r.route}</small></span>
                <span className="mono-dim">{r.blocked_by ?? (k === 'err' ? 'error' : '—')}</span>
                <span className="labpath">{(r.layers ?? []).map((l, i) => <span key={i} className={`labpath__step is-${kind(l.decision)}`} title={`${l.layer}: ${l.decision}`}>{l.layer}</span>)}{!r.layers?.length && <span className="mono-dim">—</span>}</span>
                <span className="mono-dim">{r.latency_ms != null ? `${r.latency_ms} ms` : '—'}</span>
                <span className="mono-dim">{time(r.timestamp)}</span>
              </button>
              {open === id && (
                <div className="labdetail" role="row">
                  {r.error && <p className="callout small">{r.error}</p>}
                  <ol className="story story--compact">
                    {(r.layers ?? []).map((l, i) => (
                      <li key={i} className={`story__beat tone--${kind(l.decision) === 'deny' ? 'bad' : kind(l.decision) === 'flag' ? 'warn' : 'plain'}`}>
                        <span className="story__k">{l.layer}</span>
                        <span className="story__t">
                          {l.decision}{l.reason ? ` · ${l.reason}` : ''}
                          {l.message && <small>{l.message}</small>}
                          {l.prompt_guard?.malicious_score != null && <small>malicious score {l.prompt_guard.malicious_score.toFixed(3)} · threshold {l.prompt_guard.threshold}</small>}
                        </span>
                      </li>
                    ))}
                  </ol>
                </div>
              )}
            </Fragment>
          )
        })}
        {state === 'up' && !rows.length && <p className="empty">No requests yet. Call <code>POST /v1/transactions/verify</code> or run the Solana benchmark.</p>}
      </div>
    </div>
  )
}
