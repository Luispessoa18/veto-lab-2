import { useEffect, useRef } from 'react'

export interface Frame {
  ctx: CanvasRenderingContext2D
  w: number
  h: number
  t: number
  dt: number
  pointer: { x: number; y: number; active: boolean }
}

/**
 * Sizes a canvas to its box at device pixel ratio, runs `draw` every frame
 * while the canvas is on screen, and tracks the pointer in local CSS pixels.
 * `draw` is read through a ref so callers can close over fresh props.
 */
export function useCanvasLoop(
  draw: (f: Frame) => void,
  opts: { maxDpr?: number; onResize?: (w: number, h: number) => void; paused?: boolean } = {},
) {
  const ref = useRef<HTMLCanvasElement>(null)
  const drawRef = useRef(draw)
  drawRef.current = draw
  const resizeRef = useRef(opts.onResize)
  resizeRef.current = opts.onResize
  const pausedRef = useRef(!!opts.paused)
  pausedRef.current = !!opts.paused

  useEffect(() => {
    const canvas = ref.current!
    const ctx = canvas.getContext('2d')!
    const pointer = { x: -9999, y: -9999, active: false }
    let w = 0, h = 0, raf = 0, visible = true, last = performance.now()
    const start = last

    const resize = () => {
      // Layout size, not getBoundingClientRect: a parent mid scale/translate
      // animation would otherwise freeze the canvas at the transformed size.
      const dpr = Math.min(devicePixelRatio || 1, opts.maxDpr ?? 2)
      w = canvas.offsetWidth; h = canvas.offsetHeight
      canvas.width = Math.max(1, Math.round(w * dpr))
      canvas.height = Math.max(1, Math.round(h * dpr))
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
      resizeRef.current?.(w, h)
      // A paused loop has stopped ticking, so a resize must repaint by hand.
      if (pausedRef.current) drawRef.current({ ctx, w, h, t: 99, dt: 0, pointer })
    }
    const ro = new ResizeObserver(resize)
    ro.observe(canvas)
    resize()

    const io = new IntersectionObserver(([e]) => { visible = e.isIntersecting })
    io.observe(canvas)

    const move = (e: PointerEvent) => {
      const r = canvas.getBoundingClientRect()
      pointer.x = e.clientX - r.left
      pointer.y = e.clientY - r.top
      pointer.active = pointer.x >= 0 && pointer.y >= 0 && pointer.x <= r.width && pointer.y <= r.height
    }
    const leave = () => { pointer.active = false }
    addEventListener('pointermove', move, { passive: true })
    document.addEventListener('pointerleave', leave)

    const tick = (now: number) => {
      const dt = Math.min(0.05, (now - last) / 1000)
      last = now
      // Paused (reduced motion): paint one settled frame and stop the loop.
      if (pausedRef.current) {
        drawRef.current({ ctx, w, h, t: 99, dt: 0, pointer })
        return
      }
      raf = requestAnimationFrame(tick)
      if (!visible || document.hidden) return
      drawRef.current({ ctx, w, h, t: (now - start) / 1000, dt, pointer })
    }
    raf = requestAnimationFrame(tick)

    return () => {
      cancelAnimationFrame(raf)
      ro.disconnect(); io.disconnect()
      removeEventListener('pointermove', move)
      document.removeEventListener('pointerleave', leave)
    }
  }, [])

  return ref
}
