/** Inline SVG marks from the mockup. Decorative: always aria-hidden. */
const common = { viewBox: '0 0 12 12', fill: 'none', stroke: 'currentColor', strokeWidth: 2, 'aria-hidden': true } as const

export function OkIcon() {
  return (
    <svg {...common}>
      <path d="M2.5 6.5l2.2 2.2L9.5 3.8" />
    </svg>
  )
}

export function NoIcon() {
  return (
    <svg {...common}>
      <path d="M3 3l6 6M9 3L3 9" />
    </svg>
  )
}

export function SkipIcon() {
  return (
    <svg {...common}>
      <path d="M3 6h6" />
    </svg>
  )
}
