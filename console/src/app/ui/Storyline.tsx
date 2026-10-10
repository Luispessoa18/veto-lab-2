import { motion } from 'motion/react'
import type { Entry } from '../api/types'
import { agentOf } from '../api/issues'
import { amount, EFFECT_LABEL, short } from '../api/format'
import { DECISION_WORD } from './Decision'
import { useReduced } from '../../lib/useReduced'

/*
 * A verdict told as a story, read top to bottom, the way endpoint-security
 * tools show a detection: what the agent read, what it proposed, what the
 * simulation showed, where the money would go, what the checks found, and
 * where AVAL stopped it (or let it through). Every line comes from the trace.
 */

type Tone = 'plain' | 'bad' | 'warn' | 'end-deny' | 'end-flag' | 'end-allow'
type Beat = { k: string; text: string; detail?: string; tone: Tone }

export function beats(e: Entry): Beat[] {
  const t = e.trace
  const out: Beat[] = []

  const read = t.input.chunks.filter((c) => c.provenance !== 'user')
  if (read.length) {
    for (const c of read) out.push({ k: 'read', text: `${c.source ?? c.provenance.replace(/_/g, ' ')}: “${c.text}”`, detail: c.trusted ? 'trusted source' : 'untrusted source', tone: c.trusted ? 'plain' : 'bad' })
  } else {
    out.push({ k: 'asked', text: `“${t.input.goal}”`, detail: `${agentOf(e)} · ${e.request.provenance.replace(/_/g, ' ')}`, tone: 'plain' })
  }

  const args = t.toolCall?.args ?? {}
  const amt = typeof args.amount === 'string' || typeof args.amount === 'number' ? ` · amount ${args.amount}` : ''
  out.push({ k: 'proposed', text: `${t.toolCall?.name ?? e.request.fixture}${amt}`, detail: t.toolCall?.manifest ? `manifest v${t.toolCall.manifest.version}` : 'no manifest declared', tone: t.toolCall?.manifest ? 'plain' : 'warn' })

  const changes = t.effects.filter((x) => x.type !== 'PROGRAM_INVOKE')
  const sim = t.transaction
  out.push({
    k: 'simulated',
    text: changes.length ? changes.slice(0, 3).map((x) => `${EFFECT_LABEL[x.type].toLowerCase()}${amount(x) ? ` ${amount(x)}` : ''}`).join(' · ') : 'no balance or authority changes',
    detail: `${sim.simulation}${sim.slot ? ` · slot ${sim.slot.toLocaleString('en-US')}` : ''}`,
    tone: sim.simulation === 'executed' ? 'plain' : 'warn',
  })

  for (const c of t.counterparties) {
    const bad = c.severity === 'critical' || c.severity === 'high'
    out.push({ k: 'origin', text: `${short(c.address)} came from ${c.origin.replace(/_/g, ' ')}`, detail: c.sources.length ? c.sources.join(', ') : undefined, tone: bad ? 'bad' : 'plain' })
  }

  const serious = t.findings.filter((f) => f.severity !== 'low')
  if (serious.length) for (const f of serious) out.push({ k: f.severity, text: f.rule, detail: f.check, tone: f.severity === 'critical' || f.severity === 'high' ? 'bad' : 'warn' })
  else out.push({ k: 'checks', text: 'nothing above low', tone: 'plain' })

  const d = t.decision.outcome
  const held = t.decision.steps.find((s) => s.held)
  out.push({
    k: d === 'deny' ? 'stopped' : d === 'flag' ? 'held' : 'passed',
    text: `${DECISION_WORD[d]} · ${held?.rule ?? ''}`,
    detail: d === 'deny' ? 'nothing was signed' : d === 'flag' ? 'waiting for a person' : 'signed and sent',
    tone: d === 'deny' ? 'end-deny' : d === 'flag' ? 'end-flag' : 'end-allow',
  })
  return out
}

export default function Storyline({ e, compact = false }: { e: Entry; compact?: boolean }) {
  const reduced = useReduced()
  return (
    <ol className={`story ${compact ? 'story--compact' : ''}`} aria-label="What happened, in order">
      {beats(e).map((b, i) => (
        <motion.li
          key={i}
          className={`story__beat tone--${b.tone}`}
          initial={reduced ? false : { opacity: 0, x: -6 }}
          animate={{ opacity: 1, x: 0 }}
          transition={{ delay: 0.1 + i * 0.06 }}
        >
          <span className="story__k">{b.k}</span>
          <span className="story__t">{b.text}{b.detail && <small>{b.detail}</small>}</span>
        </motion.li>
      ))}
    </ol>
  )
}
