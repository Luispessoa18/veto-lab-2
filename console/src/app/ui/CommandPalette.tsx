import { useEffect, useMemo, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'motion/react'
import { go } from '../router'
import { useEntries } from '../api/store'
import { FIXTURES } from '../api/fixtures'
import { DECISION_WORD } from './Decision'
import { current, toggleTheme } from '../../lib/theme'

/* ⌘K / Ctrl+K: jump to any page, recent verdict, or start a fixture run. */
type Cmd = { id: string; label: string; hint: string; run: () => void }

export default function CommandPalette() {
  const [open, setOpen] = useState(false)
  const [q, setQ] = useState('')
  const [i, setI] = useState(0)
  const input = useRef<HTMLInputElement>(null)
  const entries = useEntries()

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); setOpen((o) => !o) }
      if (e.key === 'Escape') setOpen(false)
    }
    const ext = () => setOpen(true)
    addEventListener('keydown', key)
    addEventListener('aval:palette', ext)
    return () => { removeEventListener('keydown', key); removeEventListener('aval:palette', ext) }
  }, [])
  useEffect(() => { if (open) { setQ(''); setI(0); setTimeout(() => input.current?.focus(), 10) } }, [open])

  const all: Cmd[] = useMemo(() => [
    { id: 'p-o', label: 'Overview', hint: 'page', run: () => go('overview') },
    { id: 'p-e', label: 'Evaluate a transaction', hint: 'page', run: () => go('evaluate') },
    { id: 'p-mo', label: 'Monitoring: activity and charts', hint: 'monitor', run: () => go('monitoring') },
    { id: 'p-a', label: 'Actions', hint: 'page', run: () => go('actions') },
    { id: 'p-r', label: 'Review queue', hint: 'page', run: () => go('review') },
    { id: 'p-ag', label: 'Agents', hint: 'operate', run: () => go('agents') },
    { id: 'p-po', label: 'Policy: thresholds, gate, judge', hint: 'configure', run: () => go('policy') },
    { id: 'p-mf', label: 'Manifests', hint: 'configure', run: () => go('manifests') },
    { id: 'p-li', label: 'Lists: allow, deny, trusted', hint: 'configure', run: () => go('lists') },
    { id: 'p-de', label: 'Deploy: publish or roll back policy', hint: 'ship', run: () => go('deploy') },
    { id: 'p-sy', label: 'System health', hint: 'ship', run: () => go('system') },
    { id: 't', label: `Switch to ${current() === 'dark' ? 'light' : 'dark'} theme`, hint: 'theme', run: () => toggleTheme(innerWidth / 2, innerHeight / 3) },
    ...FIXTURES.map((f) => ({ id: 'f-' + f.name, label: `Run ${f.title}`, hint: 'evaluate', run: () => go(`evaluate/${f.name}`) })),
    ...entries.slice(0, 12).map((e) => ({
      id: 'e-' + e.id,
      label: `${e.trace.toolCall?.name ?? e.request.fixture} · ${e.trace.transaction.messageDigest?.slice(0, 10) ?? ''}`,
      hint: DECISION_WORD[e.trace.decision.outcome],
      run: () => go(`actions/${e.id}`),
    })),
  ], [entries])

  const shown = all.filter((c) => (c.label + ' ' + c.hint).toLowerCase().includes(q.toLowerCase())).slice(0, 10)
  const pick = (c?: Cmd) => { if (!c) return; setOpen(false); c.run() }

  return (
    <AnimatePresence>
      {open && (
        <motion.div className="palette__scrim" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} onClick={() => setOpen(false)}>
          <motion.div
            className="palette"
            role="dialog"
            aria-label="Command palette"
            initial={{ opacity: 0, y: -12, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -8, scale: 0.98 }}
            transition={{ type: 'spring', stiffness: 500, damping: 36 }}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="palette__in">
              <span aria-hidden="true">›</span>
              <input
                ref={input}
                value={q}
                placeholder="Jump to a page, a verdict, or run a fixture…"
                aria-label="Search commands"
                onChange={(e) => { setQ(e.target.value); setI(0) }}
                onKeyDown={(e) => {
                  if (e.key === 'ArrowDown') { e.preventDefault(); setI((x) => Math.min(shown.length - 1, x + 1)) }
                  if (e.key === 'ArrowUp') { e.preventDefault(); setI((x) => Math.max(0, x - 1)) }
                  if (e.key === 'Enter') pick(shown[i])
                }}
              />
              <kbd>esc</kbd>
            </div>
            <ul className="palette__list" role="listbox">
              {shown.map((c, k) => (
                <li
                  key={c.id}
                  role="option"
                  aria-selected={k === i}
                  className={k === i ? 'is-on' : ''}
                  onMouseEnter={() => setI(k)}
                  onClick={() => pick(c)}
                >
                  {k === i && <motion.span layoutId="palette-hl" className="palette__hl" transition={{ type: 'spring', stiffness: 600, damping: 40 }} />}
                  <span>{c.label}</span><em>{c.hint}</em>
                </li>
              ))}
              {!shown.length && <li className="palette__empty">Nothing matches “{q}”.</li>}
            </ul>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}
