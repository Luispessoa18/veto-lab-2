import { lazy, Suspense } from 'react'
import { AnimatePresence, motion } from 'motion/react'
import { useRoute } from './router'
import { useEngine } from './api/engine'
import Overview from './pages/Overview'
import Live from './pages/Live'
import Issues from './pages/Issues'
import LabTraffic from './pages/LabTraffic'
import Evaluate from './pages/Evaluate'
import Actions from './pages/Actions'
import TraceView from './pages/TraceView'
import Review from './pages/Review'
import Policy from './pages/Policy'
import Manifests from './pages/Manifests'
import Lists from './pages/Lists'
import Agents from './pages/Agents'
import Deploy from './pages/Deploy'
import System from './pages/System'
import Inspect from './pages/Inspect'
import { ChangeBar } from './config/ui'
import { useConfig } from './config/store'
import { diff } from './config/impact'
import CommandPalette from './ui/CommandPalette'
import Toasts from './ui/Toasts'
import Scramble from '../components/Scramble'
import Logo from '../components/Logo'
import ThemeToggle from '../components/ThemeToggle'
import { useReduced } from '../lib/useReduced'

// Monitoring pulls in recharts (~half the console's bundle), so it loads on first visit.
const Monitoring = lazy(() => import('./pages/Monitoring'))

function PageLoading({ label }: { label: string }) {
  return (
    <div className="page pageload" role="status" aria-live="polite">
      <p className="kicker">loading {label}…</p>
      <div className="pageload__grid" aria-hidden="true">{Array.from({ length: 7 }, (_, i) => <span key={i} />)}</div>
    </div>
  )
}

const NAV = [
  { group: 'monitor', items: [
    { k: 'live', label: 'Live gate', g: '◉' },
    { k: 'overview', label: 'Overview', g: '◇' },
    { k: 'issues', label: 'Issues', g: '⚑' },
    { k: 'lab', label: 'Lab traffic', g: '⌗' },
    { k: 'monitoring', label: 'Monitoring', g: '∿' },
    { k: 'actions', label: 'Actions', g: '≡' },
    { k: 'review', label: 'Review', g: '◎' },
  ] },
  { group: 'operate', items: [
    { k: 'evaluate', label: 'Scenario Lab', g: '▷' },
    { k: 'agents', label: 'Agents', g: '⌬' },
  ] },
  { group: 'configure', items: [
    { k: 'policy', label: 'Policy', g: '⚖' },
    { k: 'manifests', label: 'Manifests', g: '▤' },
    { k: 'lists', label: 'Lists', g: '☰' },
  ] },
  { group: 'ship', items: [
    { k: 'deploy', label: 'Deploy', g: '⇪' },
    { k: 'system', label: 'System', g: '⌁' },
  ] },
]

export default function App() {
  const route = useRoute()
  const engine = useEngine()
  const reduced = useReduced()
  const cfg = useConfig()
  const pending = diff(cfg.history[0].config, cfg.draft).length
  const [page = 'live', arg, arg2] = route
  const key = route.join('/') || 'live'

  let view
  if (page === 'live') view = <Live />
  else if (page === 'issues') view = <Issues id={arg} />
  else if (page === 'lab') view = <LabTraffic />
  else if (page === 'evaluate') view = <Evaluate preset={arg} />
  else if (page === 'actions' && arg) view = <TraceView id={arg} />
  else if (page === 'actions') view = <Actions />
  else if (page === 'review') view = <Review />
  else if (page === 'overview' && arg === 'inspect' && arg2) view = <Inspect id={arg2} />
  else if (page === 'monitoring') view = <Suspense fallback={<PageLoading label="monitoring" />}><Monitoring /></Suspense>
  else if (page === 'policy') view = <Policy />
  else if (page === 'manifests') view = <Manifests />
  else if (page === 'lists') view = <Lists />
  else if (page === 'agents') view = <Agents />
  else if (page === 'deploy') view = <Deploy />
  else if (page === 'system') view = <System />
  else view = <Overview />

  return (
    <div className="shell">
      <a href="#main" className="skip">Skip to content</a>
      <aside className="rail">
        <a className="rail__brand" href="/" aria-label="AVAL site">
          <Logo className="rail__logo" label="AVAL" />
          <em>console</em>
        </a>
        <nav className="rail__nav" aria-label="Console">
          {NAV.map((grp) => (
            <div key={grp.group} className="rail__group">
              <span className="rail__glabel">{grp.group}</span>
              {grp.items.map((n) => {
                const on = page === n.k
                return (
                  <a key={n.k} href={`#/${n.k}`} className={on ? 'is-on' : ''} aria-current={on ? 'page' : undefined}>
                    {on && <motion.span layoutId="rail-hl" className="rail__hl" transition={{ type: 'spring', stiffness: 500, damping: 40 }} />}
                    <span className="rail__g" aria-hidden="true">{n.g}</span>
                    <Scramble text={n.label} />
                    {n.k === 'deploy' && pending > 0 && <span className="rail__badge" aria-label={`${pending} unpublished changes`}>{pending}</span>}
                  </a>
                )
              })}
            </div>
          ))}
        </nav>
        <button type="button" className="rail__k" onClick={() => dispatchEvent(new Event('aval:palette'))}>
          <span>Search</span><kbd>⌘K</kbd>
        </button>
        <ThemeToggle className="rail__theme" />
        <div className={`rail__engine engine engine--${engine}`}>
          <i aria-hidden="true" />
          <span>
            {engine === 'live' ? 'engine live' : engine === 'offline' ? 'engine offline' : 'checking…'}
            <small>{engine === 'offline' ? 'start the Veto demo server' : 'veto demo · /veto'}</small>
          </span>
        </div>
      </aside>

      <main id="main" className="main">
        <AnimatePresence mode="wait">
          <motion.div
            key={key}
            initial={reduced ? false : { opacity: 0, y: 10, filter: 'blur(4px)' }}
            animate={{ opacity: 1, y: 0, filter: 'blur(0px)' }}
            exit={reduced ? undefined : { opacity: 0, y: -6, filter: 'blur(4px)', transition: { duration: 0.15 } }}
            transition={{ type: 'spring', stiffness: 260, damping: 30 }}
          >
            {view}
          </motion.div>
        </AnimatePresence>
      </main>
      <ChangeBar />
      <CommandPalette />
      <Toasts />
    </div>
  )
}
