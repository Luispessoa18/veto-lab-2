import { useEffect, useState } from 'react'
import { AnimatePresence, motion } from 'motion/react'
import { approve, review as fetchReview, type Source } from '../api/api'
import type { ApprovalKind, Proposal, Review as R, ReviewItem } from '../api/types'
import { ago, short } from '../api/format'
import { DecisionBadge, SevChip } from '../ui/Decision'
import HoldButton from '../ui/HoldButton'
import Copy from '../ui/Copy'
import { toast } from '../ui/Toasts'
import Scramble from '../../components/Scramble'

const KIND: Record<ApprovalKind, { label: string; what: string; tone: 'ink' | 'deny' }> = {
  trust_address: { label: 'Trust address', what: 'Add to the allowlist. Takes effect on the next verdict.', tone: 'ink' },
  block_address: { label: 'Block address', what: 'Add to the deny-list. Takes effect on the next verdict.', tone: 'deny' },
  write_manifest: { label: 'Write manifest', what: 'Queue a manifest for this tool. A code change, so it waits as pending.', tone: 'ink' },
  review_manifest: { label: 'Review manifest', what: 'Queue the existing manifest for review. Waits as pending.', tone: 'ink' },
  allow_program: { label: 'Allow program', what: 'Queue the program for the allowlist. Waits as pending.', tone: 'ink' },
}

export default function Review() {
  const [data, setData] = useState<R | null>(null)
  const [source, setSource] = useState<Source>('snapshot')
  const [busy, setBusy] = useState(false)

  useEffect(() => { fetchReview().then((r) => { setData(r.review); setSource(r.source) }) }, [])

  const act = async (item: ReviewItem, p: Proposal) => {
    setBusy(true)
    try {
      const r = await approve(p.kind, item.subject, {
        considered: p.retrospect.considered,
        changed: p.retrospect.changed.length,
        speculative: p.retrospect.speculative,
      })
      setData(r.review); setSource(r.source)
      toast({
        title: `${KIND[p.kind].label}: ${short(item.subject.value, 6)}`,
        body: r.source === 'live' ? 'Recorded by the engine. Policy version moves on the next verdict.' : 'Engine offline: applied in this tab only.',
        tone: KIND[p.kind].tone === 'deny' ? 'deny' : 'ink',
      })
    } catch (e) {
      toast({ title: 'Approval refused', body: (e as Error).message, tone: 'deny' })
    } finally { setBusy(false) }
  }

  return (
    <div className="page">
      <header className="page__head">
        <Scramble as="p" text={`review · ${source === 'live' ? 'live engine' : 'recorded snapshot'}`} className="kicker" />
        <h1>Things only a human should decide.</h1>
        <p className="lede">
          Subjects that kept tripping the same rules. Each proposal shows which past verdicts it would have
          changed, so you approve knowing the consequence. Approving is press and hold.
        </p>
      </header>

      {!data ? <p className="empty">Loading queue…</p> : (
        <div className="review">
          <div className="stack">
            <AnimatePresence initial={false}>
              {data.items.map((it) => (
                <motion.article
                  key={it.subject.kind + it.subject.value}
                  layout
                  className={`ritem sevline--${it.highestSeverity} ${it.stance ? 'is-decided' : ''}`}
                  initial={{ opacity: 0, y: 10 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, x: 60, transition: { duration: 0.25 } }}
                >
                  <header className="ritem__head">
                    <span className="ritem__kind">{it.subject.kind}</span>
                    <Copy value={it.subject.value} display={it.subject.kind === 'address' ? short(it.subject.value, 8) : it.subject.value} className="ritem__subj" />
                    <SevChip s={it.highestSeverity} />
                  </header>
                  <p className="mono-dim small">
                    {it.occurrences} occurrences · first {ago(it.firstSeen)} · last {ago(it.lastSeen)}
                  </p>
                  <ul className="ritem__rules">
                    {it.rules.map((r) => <li key={r.rule}><code>{r.rule}</code><SevChip s={r.severity} n={r.count} /></li>)}
                  </ul>
                  {it.stance ? (
                    <motion.p className={`decided decided--${it.stance}`} initial={{ opacity: 0, scale: 0.98 }} animate={{ opacity: 1, scale: 1 }}>
                      <span aria-hidden="true">■</span> decided · {KIND[it.stance].label.toLowerCase()}
                      <em>{KIND[it.stance].what}</em>
                    </motion.p>
                  ) : (
                  <div className="props">
                    {it.proposals.map((p) => (
                      <div key={p.kind} className="prop">
                        <div className="prop__txt">
                          <b>{KIND[p.kind].label}</b>
                          <span>{KIND[p.kind].what}</span>
                          <span className="retro">
                            would change <b>{p.retrospect.changed.length}</b> of {p.retrospect.considered} past verdicts
                            {p.retrospect.speculative && <em> · speculative</em>}
                          </span>
                          {!!p.retrospect.changed.length && (
                            <span className="retro__list">
                              {p.retrospect.changed.slice(0, 3).map((c) => (
                                <span key={c.actionId}><DecisionBadge d={c.from} /> → <DecisionBadge d={c.to} /></span>
                              ))}
                            </span>
                          )}
                        </div>
                        <HoldButton label={KIND[p.kind].label} tone={KIND[p.kind].tone} disabled={busy} onConfirm={() => act(it, p)} />
                      </div>
                    ))}
                  </div>
                  )}
                </motion.article>
              ))}
            </AnimatePresence>
            {!data.items.length && <p className="empty">Queue is clear. Nothing has repeated often enough to need a person.</p>}
          </div>

          <aside className="stack">
            <section className="panel">
              <div className="panel__head"><h2>Live policy</h2><span className="mono-dim">what humans approved</span></div>
              <PolicyList title="allowlist" items={data.live.allowlist} />
              <PolicyList title="deny-list" items={data.live.denylist} tone="deny" />
              <p className="plist__t">pending code changes</p>
              {data.live.pending.length ? (
                <ul className="plist">{data.live.pending.map((p, i) => <li key={i}><code>{p.kind}</code> {p.subject.value}</li>)}</ul>
              ) : <p className="none">none</p>}
            </section>
            <section className="panel">
              <div className="panel__head"><h2>Unattributable</h2><span className="mono-dim">rules with no single subject</span></div>
              <ul className="ritem__rules">
                {data.unattributable.map((u) => <li key={u.rule}><code>{u.rule}</code><SevChip s={u.severity} n={u.count} /></li>)}
              </ul>
              <p className="mono-dim small">{data.records} actions on record · shown after {data.minOccurrences}+ occurrences</p>
            </section>
          </aside>
        </div>
      )}
    </div>
  )
}

function PolicyList({ title, items, tone }: { title: string; items: string[]; tone?: 'deny' }) {
  return (
    <>
      <p className="plist__t">{title}</p>
      {items.length ? (
        <ul className={`plist ${tone ? 'plist--deny' : ''}`}>
          <AnimatePresence initial={false}>
            {items.map((a) => (
              <motion.li key={a} initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: 'auto' }}>
                <Copy value={a} display={short(a, 8)} />
              </motion.li>
            ))}
          </AnimatePresence>
        </ul>
      ) : <p className="none">empty</p>}
    </>
  )
}
