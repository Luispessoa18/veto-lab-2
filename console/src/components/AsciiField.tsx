import { useEffect, useRef } from 'react'
import { useCanvasLoop } from '../lib/useCanvasLoop'
import { MONO, RAMP } from '../lib/glyphs'
import { useReduced } from '../lib/useReduced'
import { palette } from '../lib/theme'

/*
 * Full-bleed glyph field. Three interfering sinusoids pick a ramp character per
 * cell; the pointer carves a calm hole, and each click sends out a ring that
 * lights the cells it passes. Cells are batched by brightness so a frame costs
 * a handful of fillStyle switches, not one per glyph.
 */
const LEVELS = 6

export default function AsciiField({
  cell = 14,
  intensity = 0.32,
  className,
}: { cell?: number; intensity?: number; className?: string }) {
  const reduced = useReduced()
  const rings = useRef<{ x: number; y: number; t: number }[]>([])
  const buckets = useRef<{ x: number; y: number; c: string }[][]>(
    Array.from({ length: LEVELS }, () => []),
  )

  const ref = useCanvasLoop(
    ({ ctx, w, h, t, pointer }) => {
      ctx.clearRect(0, 0, w, h)
      const cw = cell * 0.62
      const cols = Math.ceil(w / cw)
      const rows = Math.ceil(h / cell)
      const aspect = w / Math.max(h, 1)
      const b = buckets.current
      for (const list of b) list.length = 0
      for (const r of rings.current) if (r.t < 0) r.t = t
      rings.current = rings.current.filter((r) => t - r.t < 2.4)

      for (let j = 0; j < rows; j++) {
        const py = j * cell + cell / 2
        const ny = (py / h) * 2 - 1
        for (let i = 0; i < cols; i++) {
          const px = i * cw + cw / 2
          const nx = ((px / w) * 2 - 1) * aspect
          let v =
            (Math.sin(nx * 2.1 + t * 0.35) +
              Math.sin(ny * 3.3 - t * 0.27) +
              Math.sin((nx + ny) * 2.6 + t * 0.5) +
              Math.sin(Math.hypot(nx * 1.4, ny + 0.6) * 5 - t * 0.9)) /
            4
          v = Math.max(0, v * 0.5 + 0.5)
          v = v * v * 1.2

          if (pointer.active) {
            const d = Math.hypot(px - pointer.x, py - pointer.y)
            v *= Math.min(1, Math.max(0, (d - 30) / 160))
          }
          for (const r of rings.current) {
            const age = t - r.t
            const d = Math.hypot(px - r.x, py - r.y)
            const band = Math.abs(d - age * 520)
            if (band < 40) v += (1 - band / 40) * (1 - age / 2.4) * 1.6
          }
          if (v < 0.08) continue
          v = Math.min(1, v)
          const ch = RAMP[Math.min(RAMP.length - 1, 1 + Math.floor(v * (RAMP.length - 2)))]
          b[Math.min(LEVELS - 1, Math.floor(v * LEVELS))].push({ x: i * cw, y: j * cell, c: ch })
        }
      }

      ctx.font = `${cell * 0.92}px ${MONO}`
      ctx.textBaseline = 'top'
      for (let k = 0; k < LEVELS; k++) {
        ctx.fillStyle = `rgba(${palette.ink},${(((k + 1) / LEVELS) * intensity).toFixed(3)})`
        for (const g of b[k]) ctx.fillText(g.c, g.x, g.y)
      }
    },
    { maxDpr: 1.5, paused: reduced },
  )

  // Clicks land on content above the canvas, so listen on the window and keep
  // only presses inside our box. The ring gets its start time on the next frame.
  useEffect(() => {
    const down = (e: PointerEvent) => {
      const r = ref.current?.getBoundingClientRect()
      if (!r) return
      const x = e.clientX - r.left, y = e.clientY - r.top
      if (x < 0 || y < 0 || x > r.width || y > r.height) return
      rings.current.push({ x, y, t: -1 })
    }
    addEventListener('pointerdown', down)
    return () => removeEventListener('pointerdown', down)
  }, [ref])

  return <canvas ref={ref} aria-hidden="true" className={className} />
}
