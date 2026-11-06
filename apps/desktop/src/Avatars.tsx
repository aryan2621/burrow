import { UserIcon } from "lucide-react";
import { cn } from "@/lib/utils";

/** Burrow's agent avatars: simple, bright characters with small faces. No frame: the shape is the character. */
const INK = "#2b211c";
const Eye = ({ x, y }: { x: number; y: number }) => (
  <><ellipse cx={x} cy={y} rx={2.4} ry={3} fill={INK} /><circle cx={x + 0.8} cy={y - 1.1} r={0.8} fill="#fff" /></>
);
const Face = ({ y, dx = 7, cheeks = true }: { y: number; dx?: number; cheeks?: boolean }) => (
  <>
    <Eye x={32 - dx} y={y} /><Eye x={32 + dx} y={y} />
    {cheeks && <><circle cx={32 - dx - 4} cy={y + 5} r={2.6} fill="#f37a7a" opacity={0.35} /><circle cx={32 + dx + 4} cy={y + 5} r={2.6} fill="#f37a7a" opacity={0.35} /></>}
    <path d={`M29.5 ${y + 6}q2.5 2.4 5 0`} fill="none" stroke={INK} strokeWidth={1.6} strokeLinecap="round" />
  </>
);

const AVATARS = {
  pebble: {
    label: "Pebble",
    art: <><path d="M30 9c15-1 26 9 26 23 0 15-10 25-25 25S8 48 8 33C8 19 16 10 30 9z" fill="#8fd6b4" /><Face y={31} /></>,
  },
  cloud: {
    label: "Cloud",
    art: <>
      <g fill="#9cc9f5"><circle cx={21} cy={38} r={11} /><circle cx={33} cy={29} r={14} /><circle cx={45} cy={38} r={10} /><rect x={19} y={37} width={28} height={13} rx={6.5} /></g>
      <g fill="none" stroke={INK} strokeWidth={1.8} strokeLinecap="round"><path d="M24 36q2.5 2.2 5 0" /><path d="M36 36q2.5 2.2 5 0" /></g>
      <circle cx={22} cy={41} r={2.6} fill="#f37a7a" opacity={0.35} /><circle cx={43} cy={41} r={2.6} fill="#f37a7a" opacity={0.35} />
      <circle cx={32.5} cy={42} r={1.4} fill={INK} />
    </>,
  },
  sprout: {
    label: "Sprout",
    art: <>
      <path d="M32 21v-7" stroke="#5fae63" strokeWidth={2.4} strokeLinecap="round" />
      <path d="M32 15c-2-6-8-8-13-7 1 5 7 8 13 7z" fill="#6fbf73" /><path d="M32 13c2-5 7-6 10-5-1 4-5 6-10 5z" fill="#86cf89" />
      <ellipse cx={32} cy={39} rx={20} ry={18} fill="#f6d365" /><Face y={37} />
    </>,
  },
  drop: {
    label: "Drop",
    art: <><path d="M32 6C24 18 12 28 12 40a20 20 0 0 0 40 0C52 28 40 18 32 6z" fill="#c3b1f2" /><Face y={39} dx={6} /></>,
  },
  shroom: {
    label: "Shroom",
    art: <>
      <rect x={19} y={29} width={26} height={27} rx={11} fill="#f6e7d2" />
      <path d="M7 33C7 19 18 9 32 9s25 10 25 24z" fill="#e8664f" />
      <g fill="#fff4ea"><circle cx={19} cy={22} r={3.2} /><circle cx={33} cy={16} r={2.6} /><circle cx={45} cy={24} r={3.4} /></g>
      <Face y={41} dx={5} cheeks={false} />
    </>,
  },
  star: {
    label: "Star",
    art: <>
      <path d="M32 9l7 15 16 2-12 11 3 16-14-8-14 8 3-16L9 26l16-2z" fill="#ffcc4d" stroke="#ffcc4d" strokeWidth={6} strokeLinejoin="round" />
      <Face y={32} dx={5} cheeks={false} />
    </>,
  },
  mallow: {
    label: "Mallow",
    art: <>
      <ellipse cx={24} cy={54} rx={5} ry={3.5} fill="#f59f86" /><ellipse cx={40} cy={54} rx={5} ry={3.5} fill="#f59f86" />
      <rect x={13} y={10} width={38} height={43} rx={17} fill="#ffb59e" /><Face y={29} />
    </>,
  },
  tri: {
    label: "Tri",
    art: <><path d="M32 13 53 49H11z" fill="#5cc5c0" stroke="#5cc5c0" strokeWidth={8} strokeLinejoin="round" /><Face y={37} dx={4.5} cheeks={false} /></>,
  },
} as const;

export type AvatarKind = keyof typeof AVATARS;
const AVATAR_KINDS = Object.keys(AVATARS) as AvatarKind[];
/** An agent without a saved choice (or with one from an older set) still gets a stable character. */
export const avatarFor = (id: number, saved?: string): AvatarKind =>
  saved && saved in AVATARS ? (saved as AvatarKind) : AVATAR_KINDS[id % AVATAR_KINDS.length];

export function Avatar({ kind, className }: { kind: AvatarKind; className?: string }) {
  const a = AVATARS[kind];
  return (
    <svg viewBox="0 0 64 64" aria-hidden="true" className={cn("size-8 shrink-0", className)}>
      {a.art}
    </svg>
  );
}

/** The person using Burrow, beside their own messages. */
export function UserAvatar({ className }: { className?: string }) {
  return (
    <span role="img" aria-label="You" className={cn("grid size-6 shrink-0 place-items-center rounded-full bg-secondary text-muted-foreground", className)}>
      <UserIcon className="size-3.5" />
    </span>
  );
}

/** A row of characters to choose from. */
export function AvatarPicker({ value, onChange }: { value: AvatarKind; onChange: (k: AvatarKind) => void }) {
  return (
    <div role="radiogroup" aria-label="Avatar" className="flex flex-wrap gap-1.5">
      {AVATAR_KINDS.map((k) => (
        <button key={k} type="button" role="radio" aria-checked={k === value} title={AVATARS[k].label} aria-label={AVATARS[k].label} onClick={() => onChange(k)}
          className={cn("rounded-xl p-1 ring-2", k === value ? "bg-accent ring-primary" : "ring-transparent hover:bg-accent")}>
          <Avatar kind={k} className="size-9" />
        </button>
      ))}
    </div>
  );
}
