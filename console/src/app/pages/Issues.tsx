import { useMemo, useState } from 'react'
import { AnimatePresence, motion } from 'motion/react'
import { useEntries } from '../api/store'
import { buckets, buildIssues, issueMarks, TEAM, useIssueMarks, type Issue, type IssueStatus } from '../api/issues'
import { ago } from '../api/format'
import { DecisionBadge, SevChip } from '../ui/Decision'
import Storyline from '../ui/Storyline'
import Scramble from '../../components/Scramble'
import { go } from '../router'

/* Issues: flagged and denied verdicts grouped by rule and agent (see api/issues). */

type View = 'active' | 'resolved' | 'ignored' | 'all'
const VIEWS: View[] = ['active', 'resolved', 'ignored', 'all']
const inView = (s: IssueStatus, v: View) =>
  v === 'all' || (v === 'active' ? s === 'open' || s === 'regressed' : s === v)

const STATUS_LABEL: Record<IssueStatus, string> = { open: 'open', regressed: 'came back', resolved: 'resolved', ignored: 'ignored' }

export default function Issues({ id }: { id?: string }) {
  const entries = useEntries()
  const marks = useIssueMarks()
  const issues = useMemo(() => buildIssues(entries, marks), [entries, marks])
  const [view, setView] = useState<View>('active')
  const key = id ? decodeURIComponent(id) : null
  const open = key ? issues.find((i) => i.key === key) : null
  const rows = issues.filter((i) => inView(i.status, view))
  // One time range for every sparkline, so trends compare across rows.
  const to = Date.now()
  const from = Math.min(to - 15 * 60_000, ...entries.map((e) => e.at))

  if (open) return <IssueDetail it={open} from={from} to={to} />

  return (
    <div className="page issues">
      <header className="page__head page__head--row">
        <div>
          <Scramble as="p" text="issues · repeated verdicts, grouped" className="kicker" />
          <h1>Issues</h1>
          <p className="issues__lede">Every flagged or denied action, grouped by the rule that weighed most and the agent that proposed it.</p>
        </div>
        <div className="seg" role="tablist" aria-label="Filter by status">
          {VIEWS.map((v) => (
            <button key={v} role="tab" aria-selected={view === v} className={view === v ? 'is-on' : ''} onClick={() => setView(v)}>
              {view === v && <motion.span layoutId="seg-issues" className="seg__hl" transition={{ type: 'spring', stiffness: 500, damping: 38 }} />}
              <span>{v}</span>
              <em>{issues.filter((i) => inView(i.status, v)).length}</em>
            </button>
          ))}
        </div>
      </header>

      <div className="table issues__table" role="table" aria-label="Issues">
        <div className="table__head" role="row">
          <span>verdict</span><span>issue</span><span>severity</span><span>events · trend</span><span>last seen</span><span>owner</span><span>status</span>
        </div>
        <AnimatePresence initial={false}>
          {rows.map((it) => (
            <motion.button
              type="button" role="row" key={it.key} layout
              className={`table__row irow irow--${it.decision} is-${it.status}`}
              initial={{ opacity: 0, x: -10 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0 }}
              onClick={() => go(`issues/${encodeURIComponent(it.key)}`)}
            >
              <span><DecisionBadge d={it.decision} /></span>
              <span className="irow__title"><code>{it.rule}</code><small>{it.agent}</small></span>
              <span><SevChip s={it.severity} /></span>
              <span className="irow__trend"><b>{it.entries.length}</b><Spark it={it} from={from} to={to} /></span>
              <span className="mono-dim">{ago(it.last)}</span>
              <span className="mono-dim">{it.owner ? `@${it.owner}` : '—'}</span>
              <span><span className={`ipill ipill--${it.status}`}>{STATUS_LABEL[it.status]}</span></span>
            </motion.button>
          ))}
        </AnimatePresence>
        {!rows.length && (
          <p className="empty">
            {issues.length ? `No ${view} issues.` : 'No issues yet. Open the Live gate and let the agents run; flagged and denied verdicts group here.'}
          </p>
        )}
      </div>
    </div>
  )
}

function Spark({ it, from, to }: { it: Issue; from: number; to: number }) {
  const b = buckets(it.entries, from, to, 24).map((x) => x.allow + x.flag + x.deny)
  const max = Math.max(1, ...b)
  const pts = b.map((v, i) => `${(i / (b.length - 1)) * 100},${18 - (v / max) * 16}`).join(' ')
  return (
    <svg className={`spark spark--${it.decision}`} viewBox="0 0 100 20" preserveAspectRatio="none" aria-hidden="true">
      <polyline points={`0,20 ${pts} 100,20`} className="spark__area" />
      <polyline points={pts} className="spark__line" />
    </svg>
  )
}

function IssueDetail({ it, from, to }: { it: Issue; from: number; to: number }) {
  const latest = it.entries[0]
  const b = buckets(it.entries, from, to, 40)
  const max = Math.max(1, ...b.map((x) => x.flag + x.deny))
  const active = it.status === 'open' || it.status === 'regressed'
  return (
    <div className="page issue">
      <a className="link link--muted back" href="#/issues">← issues</a>
      <header className="issue__head">
        <div>
          <Scramble as="p" text={`issue · ${it.agent}`} className="kicker" />
          <h1 className="issue__rule">{it.rule}</h1>
          <div className="trace__meta">
            <span><DecisionBadge d={it.decision} /></span>
            <span><SevChip s={it.severity} /></span>
            <span><b>{it.entries.length}</b> events</span>
            <span>first seen {ago(it.first)}</span>
            <span>last seen {ago(it.last)}</span>
            <span className={`ipill ipill--${it.status}`}>{STATUS_LABEL[it.status]}</span>
          </div>
        </div>
        <div className="issue__actions">
          <label className="issue__owner">
            <span>owner</span>
            <select id="issue-owner" value={it.owner ?? ''} onChange={(e) => issueMarks.assign(it.key, e.target.value || null)}>
              <option value="">unassigned</option>
              {TEAM.map((p) => <option key={p} value={p}>@{p}</option>)}
            </select>
          </label>
          {active ? (
            <>
              <button type="button" className="act" onClick={() => issueMarks.resolve(it.key)}>Resolve</button>
              <button type="button" className="act act--ghost" onClick={() => issueMarks.ignore(it.key)}>Ignore</button>
            </>
          ) : (
            <button type="button" className="act act--ghost" onClick={() => issueMarks.reopen(it.key)}>Reopen</button>
          )}
        </div>
      </header>
      {it.status === 'regressed' && <p className="callout">Resolved earlier, then it happened again.</p>}

      <section className="panel">
        <div className="panel__head"><h2>Events over time</h2><span className="mono-dim">this session</span></div>
        <div className="ibars" role="img" aria-label={`${it.entries.length} events between ${ago(from)} and now`}>
          {b.map((x, i) => (
            <span key={i} className="ibars__col">
              <i className="is-deny" style={{ height: `${(x.deny / max) * 100}%` }} />
              <i className="is-flag" style={{ height: `${(x.flag / max) * 100}%` }} />
            </span>
          ))}
        </div>
        <div className="ibars__axis"><span>{ago(from)}</span><span>now</span></div>
      </section>

      <div className="issue__grid">
        <section className="panel">
          <div className="panel__head"><h2>Latest event</h2><a className="link" href={`#/actions/${latest.id}`}>full trace →</a></div>
          <Storyline e={latest} />
        </section>
        <section className="panel">
          <div className="panel__head"><h2>All events</h2><span className="mono-dim">{it.entries.length}</span></div>
          <ul className="ievents">
            {it.entries.map((e) => (
              <li key={e.id}>
                <a href={`#/actions/${e.id}`}>
                  <DecisionBadge d={e.trace.decision.outcome} />
                  <span>{e.trace.toolCall?.name ?? e.request.fixture}</span>
                  <span className="mono-dim">{e.source === 'live' ? `${e.ms} ms` : 'recorded'}</span>
                  <span className="mono-dim">{ago(e.at)}</span>
                </a>
              </li>
            ))}
          </ul>
        </section>
      </div>
    </div>
  )
}
