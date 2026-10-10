import { useState } from 'react'
import { createPortal } from 'react-dom'
import { AnimatePresence, motion } from 'motion/react'
import { config, useConfig, type Agent } from '../config/store'
import { Choice, Enf, PageHead, Stepper } from '../config/ui'
import HoldButton from '../ui/HoldButton'
import Copy from '../ui/Copy'
import { short } from '../api/format'
import { toast } from '../ui/Toasts'
import { SiSolana } from 'react-icons/si'
import { RiRobot2Line } from 'react-icons/ri'
import { LuKeyRound, LuShieldCheck } from 'react-icons/lu'

/* What registering an agent wires together: the agent, its signing key, the
   network it acts on, and the AVAL gate in front of the signature. */
const REGISTER_ICONS = [
  { key: 'agent', icon: <RiRobot2Line /> },
  { key: 'key', icon: <LuKeyRound /> },
  { key: 'solana', icon: <SiSolana /> },
  { key: 'gate', icon: <LuShieldCheck /> },
]

const STATUS_LABEL = { active: 'active', paused: 'paused', killed: 'killed' } as const

export default function Agents() {
  const s = useConfig()
  const [open, setOpen] = useState<string | null>(null)
  const a = s.draft.agents.find((x) => x.id === open) ?? null
  const edit = (id: string, r: (x: Agent) => void) => config.edit((c) => { r(c.agents.find((x) => x.id === id)!) })
  const add = () => {
    const id = `agent-${(s.draft.agents.length + 1).toString().padStart(2, '0')}`
    config.edit((c) => { c.agents.push({ id, label: 'New agent', wallet: '', network: c.network, tools: [], onFlag: 'inherit', status: 'paused', dailyLimitSol: 10 }) })
    setOpen(id)
  }
  const tools = s.draft.manifests.map((m) => m.tool)

  return (
    <div className="page">
      <PageHead kicker="operate · agent registry" title="Agents and what they may do"
        lede="Each agent signs through the gate with its own wallet, its own tools and its own flag policy. New agents start paused."
        right={<div className="inline"><Enf /><button type="button" className="act act--primary" onClick={add}>+ Register agent</button></div>} />

      <section className="panel register">
        <div className="register__copy">
          <p className="kicker">register an agent</p>
          <h2 className="register__title">Name it, bind its key, put the gate in front.</h2>
          <p className="mono-dim">
            An agent gets an id, a signing wallet and the tools it may call. From then on every transaction it
            proposes is simulated and judged before its key is allowed to sign.
          </p>
          <ol className="register__steps">
            <li><span>01</span>agent id and label</li>
            <li><span>02</span>signing wallet, never the private key</li>
            <li><span>03</span>allowed tools, flag policy, daily limit</li>
          </ol>
          <button type="button" className="act act--primary" onClick={add}>+ Register agent</button>
        </div>
        <div className="register__demo" aria-hidden="true">
          <ol className="wire">
            {REGISTER_ICONS.map((x) => (
              <li key={x.key}><span className="wire__icon">{x.icon}</span><small>{x.key}</small></li>
            ))}
          </ol>
          <p className="wire__cap">treasury-07 · key ••••••••••••</p>
        </div>
      </section>

      <div className="table agents" role="table" aria-label="Agents">
        <div className="table__head" role="row"><span>agent</span><span>wallet</span><span>tools</span><span>on flag</span><span>daily limit</span><span>status</span></div>
        {s.draft.agents.map((x) => (
          <motion.button type="button" layout role="row" key={x.id} className={`table__row ${open === x.id ? 'is-open' : ''}`} onClick={() => setOpen(x.id)}>
            <span><b>{x.id}</b><small className="mono-dim"> {x.label}</small></span>
            <span className="mono-dim">{x.wallet ? short(x.wallet, 5) : 'not set'}</span>
            <span className="chips">{x.tools.length ? x.tools.slice(0, 3).map((t) => <code key={t} className="tchip">{t}</code>) : <span className="mono-dim">none</span>}{x.tools.length > 3 && <span className="mono-dim">+{x.tools.length - 3}</span>}</span>
            <span>{x.onFlag === 'inherit' ? <span className="mono-dim">inherit ({s.draft.gate.onFlag})</span> : x.onFlag}</span>
            <span>{x.dailyLimitSol} SOL</span>
            <span><span className={`status status--${x.status}`}><i aria-hidden="true" />{STATUS_LABEL[x.status]}</span></span>
          </motion.button>
        ))}
      </div>

      {/* Portaled: the page transition's transform would otherwise trap position: fixed. */}
      {createPortal(<AnimatePresence>
        {a && (
          <>
            <motion.div className="drawer__scrim" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} onClick={() => setOpen(null)} />
            <motion.aside className="drawer" role="dialog" aria-label={`Edit ${a.id}`}
              initial={{ x: '100%' }} animate={{ x: 0 }} exit={{ x: '100%' }} transition={{ type: 'spring', stiffness: 380, damping: 40 }}
              onKeyDown={(e) => e.key === 'Escape' && setOpen(null)}>
              <div className="drawer__head">
                <div><p className="kicker">agent</p><h2 className="drawer__title">{a.id}</h2></div>
                <button type="button" className="act act--ghost" onClick={() => setOpen(null)} aria-label="Close">esc ✕</button>
              </div>
              <div className="drawer__body">
                <label className="fld"><span>Label</span><input className="search" value={a.label} onChange={(e) => edit(a.id, (x) => { x.label = e.target.value })} /></label>
                <label className="fld"><span>Signing wallet</span>
                  <input className="search" value={a.wallet} placeholder="Fee payer address" onChange={(e) => edit(a.id, (x) => { x.wallet = e.target.value.trim() })} />
                  {a.wallet && <Copy value={a.wallet} display="copy address" className="small" />}
                </label>
                <div className="fld"><span>Network</span><Choice id={`net-${a.id}`} value={a.network} options={[{ v: 'mainnet', label: 'mainnet' }, { v: 'devnet', label: 'devnet' }]} onChange={(v) => edit(a.id, (x) => { x.network = v })} /></div>
                <div className="fld"><span>When a verdict is FLAG</span>
                  <Choice id={`flag-${a.id}`} value={a.onFlag} options={[{ v: 'inherit', label: 'inherit' }, { v: 'sign', label: 'sign' }, { v: 'hold', label: 'hold' }, { v: 'refuse', label: 'refuse' }]} onChange={(v) => edit(a.id, (x) => { x.onFlag = v })} />
                </div>
                <div className="fld"><span>Daily spend limit</span><Stepper id={`lim-${a.id}`} value={a.dailyLimitSol} min={0} max={100000} step={5} unit="SOL" onChange={(v) => edit(a.id, (x) => { x.dailyLimitSol = v })} /></div>
                <div className="fld"><span>Tools this agent may call <em className="mono-dim">· anything else is refused</em></span>
                  <div className="toolgrid">
                    {tools.map((t) => {
                      const on = a.tools.includes(t)
                      return (
                        <button type="button" key={t} aria-pressed={on} className={`tchip tchip--btn ${on ? 'is-on' : ''}`}
                          onClick={() => edit(a.id, (x) => { x.tools = on ? x.tools.filter((y) => y !== t) : [...x.tools, t] })}>{on ? '■' : '□'} {t}</button>
                      )
                    })}
                  </div>
                </div>
                <div className="fld"><span>Status</span>
                  <div className="inline">
                    <Choice id={`st-${a.id}`} value={a.status === 'killed' ? 'paused' : a.status} options={[{ v: 'active', label: 'active' }, { v: 'paused', label: 'paused' }]} onChange={(v) => edit(a.id, (x) => { x.status = v })} />
                    {a.status === 'killed' && <span className="status status--killed"><i aria-hidden="true" />killed</span>}
                  </div>
                </div>
                <div className="danger">
                  <div><b>Kill switch</b><span className="mono-dim small">Refuse every signature from this agent until a person re-enables it.</span></div>
                  <HoldButton label={a.status === 'killed' ? 'Killed' : 'Kill agent'} tone="deny" disabled={a.status === 'killed'}
                    onConfirm={() => { edit(a.id, (x) => { x.status = 'killed' }); toast({ title: `${a.id} killed in draft`, body: 'Publish the policy to make it the active version.', tone: 'deny' }) }} />
                </div>
                <button type="button" className="act act--ghost" onClick={() => { config.edit((c) => { c.agents = c.agents.filter((x) => x.id !== a.id) }); setOpen(null) }}>Remove from registry</button>
              </div>
            </motion.aside>
          </>
        )}
      </AnimatePresence>, document.body)}
    </div>
  )
}
