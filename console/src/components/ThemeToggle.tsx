import { toggleTheme, useTheme } from '../lib/theme'

/* Sun/moon drawn as glyphs to stay in the ASCII language. */
export default function ThemeToggle({ className = '' }: { className?: string }) {
  const theme = useTheme()
  const next = theme === 'dark' ? 'light' : 'dark'
  return (
    <button
      type="button"
      className={`themetoggle ${className}`}
      aria-label={`Switch to ${next} theme`}
      title={`Switch to ${next} theme`}
      onClick={(e) => {
        const r = e.currentTarget.getBoundingClientRect()
        toggleTheme(r.left + r.width / 2, r.top + r.height / 2)
      }}
    >
      <span className="themetoggle__track" aria-hidden="true">
        <span className="themetoggle__knob">{theme === 'dark' ? '☾' : '☼'}</span>
      </span>
      <span className="themetoggle__label">{theme}</span>
    </button>
  )
}
