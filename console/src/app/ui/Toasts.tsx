import { useSyncExternalStore } from 'react'
import { AnimatePresence, motion } from 'motion/react'

type Toast = { id: number; title: string; body?: string; tone?: 'ink' | 'deny' | 'flag' }
let list: Toast[] = []
const subs = new Set<() => void>()
const emit = () => subs.forEach((f) => f())
let n = 0

export function toast(t: Omit<Toast, 'id'>) {
  const id = ++n
  list = [...list, { ...t, id }].slice(-4)
  emit()
  setTimeout(() => { list = list.filter((x) => x.id !== id); emit() }, 4200)
}

export default function Toasts() {
  const items = useSyncExternalStore((f) => { subs.add(f); return () => subs.delete(f) }, () => list)
  return (
    <div className="toasts" role="status" aria-live="polite">
      <AnimatePresence>
        {items.map((t) => (
          <motion.div
            key={t.id}
            layout
            className={`toast toast--${t.tone ?? 'ink'}`}
            initial={{ opacity: 0, y: 16, scale: 0.96 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, x: 40, transition: { duration: 0.18 } }}
            transition={{ type: 'spring', stiffness: 420, damping: 32 }}
          >
            <b>{t.title}</b>
            {t.body && <span>{t.body}</span>}
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  )
}
