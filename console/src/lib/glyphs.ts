/* Glyph sets. RAMP goes from empty to dense, so index = brightness. */
export const RAMP = ' .\'`:-~=+*xo%#@'
export const NOISE = '!<>-_\\/[]{}=+*^?#%01'
export const BINARY = '01'
/** Canvas ASCII face: needs a 0.6em advance and the block/box glyph sets. */
export const MONO = '"JetBrains Mono", ui-monospace, monospace'

export const pick = (set: string) => set[(Math.random() * set.length) | 0]

export const rampAt = (v: number) =>
  RAMP[Math.max(0, Math.min(RAMP.length - 1, Math.round(v * (RAMP.length - 1))))]

export const hex = (n: number) =>
  Array.from({ length: n }, () => '0123456789abcdef'[(Math.random() * 16) | 0]).join('')
