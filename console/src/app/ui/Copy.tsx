import { useState } from 'react'

/* An address or digest that copies itself on click and says so. */
export default function Copy({ value, display, className = '' }: { value: string; display?: string; className?: string }) {
  const [ok, setOk] = useState(false)
  return (
    <button
      type="button"
      className={`copy ${className}`}
      title={value}
      onClick={() => {
        navigator.clipboard?.writeText(value).then(() => { setOk(true); setTimeout(() => setOk(false), 1100) }, () => {})
      }}
    >
      <span>{display ?? value}</span>
      <i aria-live="polite">{ok ? 'copied' : '⧉'}</i>
    </button>
  )
}
