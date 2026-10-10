import { motion } from 'motion/react'
import type { Decision, Severity } from '../api/types'
import Scramble from '../../components/Scramble'
import { useReduced } from '../../lib/useReduced'

export const DECISION_WORD: Record<Decision, string> = { allow: 'ALLOW', flag: 'FLAG', deny: 'DENY' }

export function DecisionBadge({ d, size = 'sm' }: { d: Decision; size?: 'sm' | 'md' }) {
  return <span className={`dbadge dbadge--${d} dbadge--${size}`}>{DECISION_WORD[d]}</span>
}

/* The big verdict on a trace: decodes, then lands with a stamp-like overshoot. */
export function DecisionStamp({ d }: { d: Decision }) {
  const reduced = useReduced()
  return (
    <motion.div
      className={`stamp stamp--${d}`}
      initial={reduced ? false : { scale: 1.6, opacity: 0, rotate: -6 }}
      animate={{ scale: 1, opacity: 1, rotate: -2 }}
      transition={{ type: 'spring', stiffness: 520, damping: 22, delay: 0.55 }}
    >
      <Scramble text={DECISION_WORD[d]} hover={false} delay={500} speed={34} />
    </motion.div>
  )
}

const SEV_ORDER: Severity[] = ['critical', 'high', 'medium', 'low']
export function SevChip({ s, n }: { s: Severity; n?: number }) {
  return <span className={`sev sev--${s}`}><i aria-hidden="true" />{s}{n != null && <b>{n}</b>}</span>
}
export function SevCounts({ counts }: { counts: Record<Severity, number> }) {
  return (
    <span className="sevcounts">
      {SEV_ORDER.map((s) => (
        <span key={s} className={`sevcount sev--${s} ${counts[s] ? '' : 'is-zero'}`} title={`${counts[s]} ${s}`}>
          <i aria-hidden="true" />{counts[s]}
        </span>
      ))}
    </span>
  )
}
