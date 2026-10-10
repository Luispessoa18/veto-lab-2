import { useEffect, useMemo, useState } from 'react'
import { motion } from 'motion/react'
import { useEntries } from '../api/store'
import { review as fetchReview } from '../api/api'
import type { Entry, Severity } from '../api/types'
import { useTheme } from '../../lib/theme'
import { PageHead } from '../config/ui'
import { ActivityBlue } from '@/components/ui/mono-activity-blue'
import { MonoArea, MonoBars, MonoComposed, MonoDonut, MonoLine, MonoScatter } from '@/components/ui/mono/charts'
import { go } from '../router'

/*
 * Monitoring: the Monocharts set, fed with this console's real verdicts.
 * Nothing here is generated; when there is not enough history a chart says so.
 */
const SEV_W: Record<Severity, number> = { critical: 8, high: 4, medium: 2, low: 1 }
const DAY = 86_400_000
const dayKey = (t: number) => new Date(t).toISOString().slice(0, 10)

function heatmap(entries: Entry[], weeks = 20) {
  const counts = new Map<string, number>()
  for (const e of entries) counts.set(dayKey(e.at), (counts.get(dayKey(e.at)) ?? 0) + 1)
  // End on today, start on the Sunday that makes a whole number of weeks.
  const end = new Date(); end.setHours(0, 0, 0, 0)
  const start = new Date(end.getTime() - (weeks * 7 - 1) * DAY)
  return Array.from({ length: weeks * 7 }, (_, i) => {
    const date = dayKey(start.getTime() + i * DAY)
    const count = counts.get(date) ?? 0
    const level = (count === 0 ? 0 : count < 2 ? 1 : count < 4 ? 2 : count < 8 ? 3 : 4) as 0 | 1 | 2 | 3 | 4
    return { date, count, level }
  })
}

/* Split the session into up to n time buckets (equal-count when everything
   happened within minutes, so the line still has shape). */
function buckets(entries: Entry[], n = 8) {
  const sorted = [...entries].sort((a, b) => a.at - b.at)
  if (!sorted.length) return []
  const size = Math.max(1, Math.ceil(sorted.length / n))
  const out: { label: string; value: number; secondary: number }[] = []
  for (let i = 0; i < sorted.length; i += size) {
    const chunk = sorted.slice(i, i + size)
    const t = new Date(chunk[chunk.length - 1].at)
    out.push({
      label: t.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false }),
      value: chunk.length,
      secondary: chunk.filter((e) => e.trace.decision.outcome !== 'allow').length,
    })
  }
  return out
}

export default function Monitoring() {
  const entries = useEntries()
  const theme = useTheme()
  const [records, setRecords] = useState<number | null>(null)
  useEffect(() => { fetchReview().then((r) => setRecords(r.review.records)).catch(() => {}) }, [])

  const m = useMemo(() => {
    const live = entries.filter((e) => e.source === 'live' && e.ms > 0).sort((a, b) => a.at - b.at)
    const ms = live.map((e) => e.ms).sort((a, b) => a - b)
    const p50 = ms.length ? ms[Math.floor((ms.length - 1) / 2)] : null

    const byTool = new Map<string, { value: number; secondary: number }>()
    for (const e of entries) {
      const k = e.trace.toolCall?.name ?? e.request.fixture
      const v = byTool.get(k) ?? { value: 0, secondary: 0 }
      v.value++; if (e.trace.decision.outcome === 'deny') v.secondary++
      byTool.set(k, v)
    }
    const tools = [...byTool.entries()].sort((a, b) => b[1].value - a[1].value).slice(0, 6)
      .map(([k, v]) => ({ label: k.replace(/_/g, ' ').replace('fund token account', 'fund acct').slice(0, 10), ...v }))

    const decisions = (['allow', 'flag', 'deny'] as const).map((d) => ({ name: d, value: entries.filter((e) => e.trace.decision.outcome === d).length })).filter((d) => d.value > 0)

    const checks = new Map<string, { bar: number; actions: Set<string> }>()
    for (const e of entries) for (const f of e.trace.findings) {
      const v = checks.get(f.check) ?? { bar: 0, actions: new Set() }
      v.bar++; v.actions.add(e.id); checks.set(f.check, v)
    }
    const checkData = [...checks.entries()].sort((a, b) => b[1].bar - a[1].bar).slice(0, 6).map(([k, v]) => ({ label: k, bar: v.bar, line: v.actions.size }))

    const scatter = entries.map((e) => ({
      x: e.trace.effects.length,
      y: e.trace.findings.length,
      z: e.trace.findings.reduce((s, f) => s + SEV_W[f.severity], 0) || 1,
      label: e.trace.toolCall?.name ?? e.request.fixture,
    }))

    const stopped = entries.filter((e) => e.trace.decision.outcome !== 'allow').length
    const findings = entries.reduce((s, e) => s + e.trace.findings.length, 0)
    const topTool = tools[0]
    return { live, p50, tools, decisions, checkData, scatter, stopped, findings, topTool, timeline: buckets(entries) }
  }, [entries])

  const total = entries.length
  const pct = (n: number) => (total ? Math.round((n / total) * 100) : 0)
  const days = new Set(entries.map((e) => dayKey(e.at))).size

  return (
    <div className="page">
      <PageHead
        kicker="monitor · activity"
        title="What the gate has been doing"
        lede={<>Every chart is computed from this console's verdict log: {total} actions ({entries.filter((e) => e.source === 'live').length} from the live engine){records != null && <>, and the engine holds {records} records</>}. Nothing is sampled or generated.</>}
        right={<button type="button" className="act act--primary" onClick={() => go('evaluate')}>Run a scenario →</button>}
      />

      <motion.div className="monitor" initial="hidden" animate="show" variants={{ show: { transition: { staggerChildren: 0.06 } } }}>
        <motion.div className="monitor__wide ui-reset" variants={rise}>
          <ActivityBlue
            theme={theme}
            contributions={heatmap(entries)}
            title="Gate activity"
            badge="Evaluations / day"
            unit="evaluations"
            footerLeft={`20 weeks · ${days} active day${days === 1 ? '' : 's'}`}
            footerRight={`${m.stopped} stopped`}
            itemLabel={['evaluation', 'evaluations']}
          />
        </motion.div>

        <div className="monitor__grid ui-reset">
          <motion.div variants={rise}>
            <MonoLine theme={theme} title="Gate throughput" badge="Line" value={total} unit="actions"
              data={m.timeline} names={['Evaluated', 'Stopped']}
              footerLeft="evaluated vs stopped" footerRight={`${pct(m.stopped)}% stopped`} />
          </motion.div>
          <motion.div variants={rise}>
            <MonoBars theme={theme} title="By tool" badge="Pillars" value={m.tools.length} unit="tools seen"
              data={m.tools} names={['Actions', 'Denied']}
              footerLeft="actions vs denied" footerRight={m.topTool ? `top · ${m.topTool.label}` : '—'} />
          </motion.div>
          <motion.div variants={rise}>
            <MonoArea theme={theme} title="Engine round trip" badge="Live runs" value={m.p50 != null ? `${m.p50}` : '—'} unit="ms p50"
              data={m.live.map((e, i) => ({ label: `#${i + 1}`, value: e.ms }))} name="Round trip (ms)"
              footerLeft="console → engine → console" footerRight={m.live.length ? `${m.live.length} live runs` : 'no live runs yet'} />
          </motion.div>
          <motion.div variants={rise}>
            <MonoDonut theme={theme} title="Decision mix" badge="Verdicts" value={`${pct(m.stopped)}%`} unit="stopped"
              data={m.decisions} centerLabel="verdicts"
              footerLeft="allow · flag · deny" footerRight={`${total} total`} />
          </motion.div>
          <motion.div variants={rise}>
            <MonoComposed theme={theme} title="Findings by check" badge="Pill + Line" value={m.findings} unit="findings"
              data={m.checkData} names={['Findings', 'Actions hit']}
              footerLeft="findings vs actions affected" footerRight={m.checkData[0] ? `most · ${m.checkData[0].label}` : '—'} />
          </motion.div>
          <motion.div variants={rise}>
            <MonoScatter theme={theme} title="Effects vs findings" badge="Per action" value={m.scatter.length} unit="actions mapped"
              data={m.scatter} xName="Effects" yName="Findings"
              footerLeft="size = severity weight" footerRight={m.scatter.length ? `max ${Math.max(...m.scatter.map((s) => s.y))} findings` : '—'} />
          </motion.div>
        </div>
      </motion.div>
    </div>
  )
}

const rise = { hidden: { opacity: 0, y: 14, filter: 'blur(6px)' }, show: { opacity: 1, y: 0, filter: 'blur(0px)', transition: { type: 'spring' as const, stiffness: 260, damping: 28 } } }
