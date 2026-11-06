type P = { className?: string };

/** The burrow mark: a tunnel mouth in the ground. */
export const Mark = ({ className = "size-6" }: P) => (
  <svg viewBox="0 0 32 32" aria-hidden="true" className={className}>
    <path d="M4 26h24" stroke="currentColor" strokeWidth={2.25} strokeLinecap="round" />
    <path d="M8.5 26v-6a7.5 7.5 0 0 1 15 0v6" fill="var(--color-amber-400)" stroke="currentColor" strokeWidth={2.25} strokeLinejoin="round" />
    <path d="M13 26v-5a3 3 0 0 1 6 0v5Z" fill="currentColor" />
  </svg>
);
