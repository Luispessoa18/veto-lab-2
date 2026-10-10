import { useEffect, useRef, useState } from 'react'
import { PageHead } from '../config/ui'
import { useConfig } from '../config/store'

/* Services behind the console, pinged through the dev proxy every 5 s. */
const SERVICES = [
  { id: 'veto', name: 'Veto engine', path: '/veto/review', target: '127.0.0.1:5173', role: 'Normalizes, simulates and judges every action; owns the review queue. Not part of this repository.' },
  { id: 'svm', name: 'aval-svm', path: '/svm/health', target: '127.0.0.1:8899', role: 'Local LiteSVM simulator (svm/) speaking JSON-RPC simulateTransaction. Start: svm/target/release/aval-svm serve.' },
  { id: 'lab', name: 'Lab API', path: '/lab/health', target: '127.0.0.1:8070', role: 'This repository’s API (src/unified_api.py): blacklist, Prompt Guard, AI, simulation. Its traffic is on the Lab traffic page.' },
] as const
const BARS = '▁▂▃▄▅▆▇█'

type Sample = { ok: boolean; ms: number }

export default function System() {
  const s = useConfig()
  const [h, setH] = useState<Record<string, Sample[]>>({})
  const alive = useRef(true)
  useEffect(() => {
    alive.current = true
    const tick = () => SERVICES.forEach(async (sv) => {
      const t0 = performance.now()
      let ok = false
      try { ok = (await fetch(sv.path, { signal: AbortSignal.timeout(3000) })).ok } catch { ok = false }
      const ms = Math.round(performance.now() - t0)
      if (alive.current) setH((prev) => ({ ...prev, [sv.id]: [...(prev[sv.id] ?? []), { ok, ms }].slice(-32) }))
    })
    tick()
    const t = setInterval(tick, 5000)
    return () => { alive.current = false; clearInterval(t) }
  }, [])

  return (
    <div className="page">
      <PageHead kicker="system · services" title="What the console is talking to" lede="Health is checked from this browser through the console's proxy, every five seconds." />
      <div className="svcgrid">
        {SERVICES.map((sv) => {
          const list = h[sv.id] ?? []
          const last = list[list.length - 1]
          const okMs = list.filter((x) => x.ok).map((x) => x.ms)
          const max = Math.max(1, ...okMs)
          const up = list.length ? Math.round((list.filter((x) => x.ok).length / list.length) * 100) : null
          const state = !last ? 'checking' : last.ok ? 'up' : 'down'
          return (
            <section key={sv.id} className={`panel svc svc--${state}`}>
              <div className="panel__head"><h2>{sv.name}</h2><span className={`svc__state`}><i aria-hidden="true" />{state}</span></div>
              <p className="mono-dim small">{sv.role}</p>
              <div className="svc__spark" aria-label={`Latency history for ${sv.name}`}>
                {list.map((x, i) => <span key={i} className={x.ok ? '' : 'is-fail'}>{x.ok ? BARS[Math.min(7, Math.floor((x.ms / max) * 7))] : '×'}</span>)}
                {!list.length && <span className="mono-dim">waiting for first check…</span>}
              </div>
              <dl className="kv">
                <dt>endpoint</dt><dd><code>{sv.target}</code></dd>
                <dt>last</dt><dd>{last ? (last.ok ? `${last.ms} ms` : 'no answer') : '—'}</dd>
                <dt>uptime</dt><dd>{up == null ? '—' : `${up}% of ${list.length} checks`}</dd>
              </dl>
              {state === 'down' && <p className="callout small">Not running. Start it locally to use it from the console.</p>}
            </section>
          )
        })}
      </div>
      <section className="panel envpanel">
        <div className="panel__head"><h2>Environment</h2></div>
        <dl className="kv">
          <dt>policy</dt><dd><code>{s.history[0].id}</code> active · {s.history.length} versions</dd>
          <dt>network</dt><dd>{s.history[0].config.network}</dd>
          <dt>RPC</dt><dd className="mono-dim">kept server-side by the engine; never sent to the browser</dd>
          <dt>proxy</dt><dd><code>/veto</code> · <code>/svm</code> · <code>/lab</code> (vite.config.ts, override with VETO_URL)</dd>
        </dl>
      </section>
    </div>
  )
}
