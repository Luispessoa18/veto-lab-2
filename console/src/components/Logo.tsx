/*
 * The AVAL lockup (figure + wordmark) as traced vectors in public/brand/.
 * Dark and light are separate drawings (their facets differ), so both ship and
 * CSS shows the one that matches the theme, with no flash on switch.
 */
export default function Logo({ className = '', label = 'AVAL' }: { className?: string; label?: string }) {
  return (
    <span className={`logo ${className}`} role="img" aria-label={label}>
      <img src="/brand/aval-lockup-dark.svg" alt="" className="logo__dark" draggable={false} />
      <img src="/brand/aval-lockup-light.svg" alt="" className="logo__light" draggable={false} />
    </span>
  )
}
