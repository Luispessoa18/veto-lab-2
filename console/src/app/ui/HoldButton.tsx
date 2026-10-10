import { useEffect, useRef, useState } from 'react'

/*
 * Press-and-hold to confirm. Policy approvals change what the gate signs, so a
 * stray click should not be enough. Space/Enter held on the keyboard works the
 * same way; releasing early rewinds the fill.
 */
export default function HoldButton({
  label,
  onConfirm,
  ms = 900,
  tone = 'ink',
  disabled,
}: {
  label: string
  onConfirm: () => void
  ms?: number
  tone?: 'ink' | 'deny'
  disabled?: boolean
}) {
  const [p, setP] = useState(0)
  const raf = useRef(0)
  const start = useRef(0)
  const done = useRef(false)

  const begin = () => {
    if (disabled) return
    done.current = false
    start.current = performance.now() - p * ms
    cancelAnimationFrame(raf.current)
    const step = (now: number) => {
      const k = Math.min(1, (now - start.current) / ms)
      setP(k)
      if (k >= 1) { done.current = true; onConfirm(); setTimeout(() => setP(0), 500); return }
      raf.current = requestAnimationFrame(step)
    }
    raf.current = requestAnimationFrame(step)
  }
  const end = () => {
    if (done.current) return
    cancelAnimationFrame(raf.current)
    const from = p, t0 = performance.now()
    const back = (now: number) => {
      const k = Math.min(1, (now - t0) / 220)
      setP(from * (1 - k))
      if (k < 1) raf.current = requestAnimationFrame(back)
    }
    raf.current = requestAnimationFrame(back)
  }
  useEffect(() => () => cancelAnimationFrame(raf.current), [])

  return (
    <button
      type="button"
      className={`hold hold--${tone} ${p >= 1 ? 'is-done' : ''}`}
      disabled={disabled}
      onPointerDown={begin}
      onPointerUp={end}
      onPointerLeave={end}
      onKeyDown={(e) => { if ((e.key === ' ' || e.key === 'Enter') && !e.repeat) { e.preventDefault(); begin() } }}
      onKeyUp={(e) => { if (e.key === ' ' || e.key === 'Enter') end() }}
      aria-label={`${label} (press and hold)`}
    >
      <span className="hold__fill" style={{ transform: `scaleX(${p})` }} aria-hidden="true" />
      <span className="hold__label">{p >= 1 ? 'done' : p > 0 ? 'hold…' : label}</span>
    </button>
  )
}
