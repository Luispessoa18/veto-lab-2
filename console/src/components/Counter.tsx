import { useEffect, useRef, useState } from 'react'
import { animate, useInView } from 'motion/react'
import { useReduced } from '../lib/useReduced'

/*
 * Counts to `to` when it scrolls into view. Digits not yet settled flicker
 * through random values so the number reads as being computed, not tweened.
 */
export default function Counter({
  to,
  decimals = 0,
  duration = 1.6,
  suffix = '',
  className,
}: {
  to: number
  decimals?: number
  duration?: number
  suffix?: string
  className?: string
}) {
  const reduced = useReduced()
  const ref = useRef<HTMLSpanElement>(null)
  const inView = useInView(ref, { once: true, amount: 0.6 })
  const final = to.toFixed(decimals)
  const [txt, setTxt] = useState(reduced ? final : final.replace(/\d/g, '0'))

  useEffect(() => {
    if (reduced) return setTxt(final)
    if (!inView) return
    const c = animate(0, 1, {
      duration,
      ease: [0.16, 1, 0.3, 1],
      onUpdate: (p) => {
        const v = (to * p).toFixed(decimals)
        // Trailing digits stay noisy until the last stretch of the tween.
        const noisy = Math.floor((1 - p) * v.length)
        setTxt(
          v
            .split('')
            .map((ch, i) => (/\d/.test(ch) && i >= v.length - noisy ? String((Math.random() * 10) | 0) : ch))
            .join(''),
        )
      },
      onComplete: () => setTxt(final),
    })
    return () => c.stop()
  }, [inView, reduced, final])

  return (
    <span ref={ref} className={className} aria-label={final + suffix}>
      <span aria-hidden="true">{txt}{suffix}</span>
    </span>
  )
}
