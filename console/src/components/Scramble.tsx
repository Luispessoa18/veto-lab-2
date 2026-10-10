import { useEffect, useRef, useState, type ComponentType, type ElementType } from 'react'
import { NOISE, pick } from '../lib/glyphs'
import { useReduced } from '../lib/useReduced'

/*
 * Text that decodes left to right: each character cycles noise until its turn,
 * then locks. Runs when it scrolls into view and again on hover or focus.
 * Screen readers get the real string through aria-label; glyphs are hidden.
 */
export default function Scramble({
  text,
  as: Tag = 'span',
  className,
  speed = 28,
  hover = true,
  delay = 0,
  href,
}: {
  text: string
  as?: ElementType
  className?: string
  /** ms between frames */
  speed?: number
  hover?: boolean
  delay?: number
  href?: string
}) {
  const reduced = useReduced()
  const [out, setOut] = useState(reduced ? text : text.replace(/\S/g, ' '))
  const el = useRef<HTMLElement>(null)
  const timer = useRef<number>(0)

  const run = () => {
    if (reduced) return setOut(text)
    clearInterval(timer.current)
    let frame = 0
    const total = text.length * 2 + 6
    timer.current = window.setInterval(() => {
      frame++
      setOut(
        text
          .split('')
          .map((ch, i) => (ch === ' ' || frame / 2 > i + 3 ? ch : frame / 2 > i - 4 ? pick(NOISE) : ' '))
          .join(''),
      )
      if (frame >= total) { clearInterval(timer.current); setOut(text) }
    }, speed)
  }

  useEffect(() => {
    if (reduced) return setOut(text)
    const io = new IntersectionObserver(([e]) => {
      if (e.isIntersecting) { setTimeout(run, delay); io.disconnect() }
    }, { threshold: 0.4 })
    io.observe(el.current!)
    return () => { io.disconnect(); clearInterval(timer.current) }
  }, [text, reduced])

  // Cast: three-fiber widens JSX intrinsic elements, which collapses a
  // polymorphic `as` tag's props to never.
  const T = Tag as ComponentType<any>
  return (
    <T
      ref={el}
      className={className}
      href={href}
      aria-label={text}
      onMouseEnter={hover ? run : undefined}
      onFocus={hover ? run : undefined}
    >
      <span aria-hidden="true">{out}</span>
    </T>
  )
}
