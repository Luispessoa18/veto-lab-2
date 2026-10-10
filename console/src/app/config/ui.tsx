import { useEffect, useRef, useState, type ReactNode } from 'react'
import { AnimatePresence, motion } from 'motion/react'
import { config, useConfig } from './store'
import { diff } from './impact'
import { go } from '../router'

/* "live" = the running engine reads this today; "staged" = versioned policy
   the console keeps until the engine's config API exists. */
export function Enf({ live }: { live?: boolean }) {
  return (
    <span className={`enf enf--${live ? 'live' : 'staged'}`} title={live ? 'Sent to the engine with every evaluation' : 'Saved as versioned policy; the engine uses its defaults until the config API (backlog F3) ships'}>
      <i aria-hidden="true" />{live ? 'live' : 'staged'}
    </span>
  )
}

export function Row({ label, hint, children, live, changed }: { label: string; hint?: string; children: ReactNode; live?: boolean; changed?: boolean }) {
  return (
    <div className={`cfgrow ${changed ? 'is-changed' : ''}`}>
      <div className="cfgrow__txt">
        <span className="cfgrow__label">{label} {changed && <em className="dirty" title="Changed in draft">●</em>}</span>
        {hint && <span className="cfgrow__hint">{hint}</span>}
      </div>
      <div className="cfgrow__ctl">{children}</div>
      <Enf live={live} />
    </div>
  )
}

export function Stepper({ value, min = 0, max = 99, step = 1, onChange, unit, id }: { value: number; min?: number; max?: number; step?: number; onChange: (v: number) => void; unit?: string; id: string }) {
  const set = (v: number) => onChange(Math.max(min, Math.min(max, v)))
  return (
    <div className="stepper">
      <button type="button" aria-label="Decrease" onClick={() => set(value - step)} disabled={value <= min}>−</button>
      <input id={id} inputMode="numeric" value={value} onChange={(e) => { const n = Number(e.target.value.replace(/[^\d]/g, '')); if (!Number.isNaN(n)) set(n) }} aria-label={unit ? `value in ${unit}` : 'value'} />
      <button type="button" aria-label="Increase" onClick={() => set(value + step)} disabled={value >= max}>+</button>
      {unit && <span className="stepper__unit">{unit}</span>}
    </div>
  )
}

export function Switch({ on, onChange, label }: { on: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button type="button" role="switch" aria-checked={on} aria-label={label} className={`switch ${on ? 'is-on' : ''}`} onClick={() => onChange(!on)}>
      <motion.span className="switch__knob" layout transition={{ type: 'spring', stiffness: 600, damping: 34 }} />
    </button>
  )
}

export function Choice<T extends string>({ value, options, onChange, id }: { value: T; options: { v: T; label: string }[]; onChange: (v: T) => void; id: string }) {
  return (
    <div className="seg" role="radiogroup">
      {options.map((o) => (
        <button type="button" role="radio" key={o.v} aria-checked={value === o.v} className={value === o.v ? 'is-on' : ''} onClick={() => onChange(o.v)}>
          {value === o.v && <motion.span layoutId={`choice-${id}`} className="seg__hl" transition={{ type: 'spring', stiffness: 500, damping: 38 }} />}
          <span>{o.label}</span>
        </button>
      ))}
    </div>
  )
}

/* Sticky bar on every page while the draft differs from the published version. */
export function ChangeBar() {
  const s = useConfig()
  const changes = diff(s.history[0].config, s.draft)
  const n = changes.length
  const prev = useRef(n)
  const [bump, setBump] = useState(0)
  useEffect(() => { if (n !== prev.current) { setBump((b) => b + 1); prev.current = n } }, [n])
  return (
    <div className="changebar-wrap">
    <AnimatePresence>
      {n > 0 && (
        <motion.div
          className="changebar"
          initial={{ y: 80, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          exit={{ y: 80, opacity: 0 }}
          transition={{ type: 'spring', stiffness: 420, damping: 34 }}
          role="region"
          aria-label="Unpublished changes"
        >
          <motion.span key={bump} className="changebar__n" initial={{ scale: 1.4 }} animate={{ scale: 1 }}>{n}</motion.span>
          <span className="changebar__txt">
            unpublished change{n === 1 ? '' : 's'} in draft
            <small>{changes.filter((c) => c.live).length} take effect live · on top of {s.history[0].id}</small>
          </span>
          <button type="button" className="act" onClick={() => config.discard()}>Discard</button>
          <button type="button" className="act act--primary" onClick={() => go('deploy')}>Review &amp; publish →</button>
        </motion.div>
      )}
    </AnimatePresence>
    </div>
  )
}

export function PageHead({ kicker, title, lede, right }: { kicker: string; title: string; lede?: ReactNode; right?: ReactNode }) {
  return (
    <header className="page__head page__head--row">
      <div>
        <p className="kicker">{kicker}</p>
        <h1>{title}</h1>
        {lede && <p className="lede">{lede}</p>}
      </div>
      {right}
    </header>
  )
}
