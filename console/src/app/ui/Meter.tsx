import { useEffect, useState } from 'react'
import { useReduced } from '../../lib/useReduced'

/* A ratio drawn in block glyphs that fills from empty when mounted. */
const PART = ' ▏▎▍▌▋▊▉'
export default function Meter({ value, cols = 24, className = '' }: { value: number; cols?: number; className?: string }) {
  const reduced = useReduced()
  const [p, setP] = useState(reduced ? value : 0)
  useEffect(() => {
    if (reduced) return setP(value)
    let raf = 0
    const t0 = performance.now(), from = 0
    const step = (now: number) => {
      const k = Math.min(1, (now - t0) / 900)
      setP(from + (value - from) * (1 - Math.pow(1 - k, 3)))
      if (k < 1) raf = requestAnimationFrame(step)
    }
    raf = requestAnimationFrame(step)
    return () => cancelAnimationFrame(raf)
  }, [value, reduced])
  const exact = Math.max(0, Math.min(1, p)) * cols
  const full = Math.floor(exact)
  const rest = full < cols ? PART[Math.round((exact - full) * 7)] : ''
  return (
    <span className={`meter ${className}`} aria-hidden="true">
      <span className="meter__on">{'█'.repeat(full)}{rest.trim()}</span>
      <span className="meter__off">{'░'.repeat(Math.max(0, cols - full - (rest.trim() ? 1 : 0)))}</span>
    </span>
  )
}
