import { useEffect, useRef, useState, type ReactNode } from "react";

/*
 * The pieces of the bar above the views: which view, and the view's own
 * tools. Tools are quiet icon buttons that say what they do on hover; a
 * pressed one takes the accent, which is the only colour the bar uses.
 */

/** Pick one of a few, like the Mac's segmented control. */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  label: string;
}) {
  return (
    <div className="segmented-control" role="tablist" aria-label={label}>
      {options.map((o) => (
        <button
          key={o.value}
          role="tab"
          aria-selected={o.value === value}
          className={o.value === value ? "on" : ""}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function ToolButton({
  title,
  pressed,
  onClick,
  children,
  disabled,
}: {
  title: string;
  /** Present for a toggle; absent for a plain action. */
  pressed?: boolean;
  onClick: () => void;
  children: ReactNode;
  disabled?: boolean;
}) {
  return (
    <button
      className={`tool-btn${pressed ? " on" : ""}`}
      title={title}
      aria-label={title}
      aria-pressed={pressed}
      onClick={onClick}
      disabled={disabled}
    >
      {children}
    </button>
  );
}

/** Close on a press outside `ref` or on Escape, while `open`. */
function useDismiss(open: boolean, ref: React.RefObject<HTMLElement | null>, close: () => void) {
  useEffect(() => {
    if (!open) return;
    const outside = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) close();
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("pointerdown", outside, true);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("keydown", esc);
    };
  }, [open, ref, close]);
}

/**
 * An icon button with a panel under it. The panel stays mounted while
 * closed, so what is inside keeps its state (and applies saved settings at
 * startup) without being opened.
 */
export function Popover({ title, icon, children }: { title: string; icon: ReactNode; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);
  useDismiss(open, ref, () => setOpen(false));
  return (
    <div className="popover" ref={ref}>
      <button
        className={`tool-btn${open ? " open" : ""}`}
        title={title}
        aria-label={title}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        {icon}
      </button>
      <div className="popover-panel" role="dialog" aria-label={title} hidden={!open}>
        <div className="popover-title">{title}</div>
        {children}
      </div>
    </div>
  );
}

export interface ShowOption {
  key: string;
  label: string;
  title: string;
  on: boolean;
}

/** What to draw alongside the notes, as a short checklist under one button. */
export function ShowMenu({
  label,
  options,
  onToggle,
}: {
  label: string;
  options: ShowOption[];
  onToggle: (key: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);
  useDismiss(open, ref, () => setOpen(false));
  const count = options.filter((o) => o.on).length;
  return (
    <div className="show-menu" ref={ref}>
      <button
        className={`tool-btn with-label${open ? " open" : ""}`}
        title={label}
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <LayersIcon />
        <span className="show-menu-label">{label}</span>
        {count > 0 && <span className="show-menu-count">{count}</span>}
        <ChevronIcon />
      </button>
      {open && (
        <div className="show-menu-pop" role="menu">
          {options.map((o) => (
            <button
              key={o.key}
              role="menuitemcheckbox"
              aria-checked={o.on}
              className="show-menu-item"
              title={o.title}
              onClick={() => onToggle(o.key)}
            >
              <span className={`show-menu-check${o.on ? " on" : ""}`}>{o.on && <CheckIcon />}</span>
              {o.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/* ---------- icons (stroked, 24-unit grid, like the panel toggles) ---------- */

const stroke = {
  width: 16,
  height: 16,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.6,
  strokeLinecap: "round",
  strokeLinejoin: "round",
  "aria-hidden": true,
} as const;

export function FollowIcon() {
  // The playhead, with the view moving along with it.
  return (
    <svg {...stroke}>
      <path d="M8 4v16" />
      <path d="M13 9l3 3-3 3M16 12h4" />
    </svg>
  );
}

export function ZoomInIcon() {
  return (
    <svg {...stroke}>
      <circle cx="10.5" cy="10.5" r="6" />
      <path d="M15 15l5 5M8 10.5h5M10.5 8v5" />
    </svg>
  );
}

export function ZoomOutIcon() {
  return (
    <svg {...stroke}>
      <circle cx="10.5" cy="10.5" r="6" />
      <path d="M15 15l5 5M8 10.5h5" />
    </svg>
  );
}

export function FitIcon() {
  // Both ends of the song pulled into view.
  return (
    <svg {...stroke}>
      <path d="M4 5v14M20 5v14" />
      <path d="M8 12h8M10.5 9.5L8 12l2.5 2.5M13.5 9.5L16 12l-2.5 2.5" />
    </svg>
  );
}

export function KeysIcon() {
  return (
    <svg {...stroke}>
      <rect x="3.5" y="5.5" width="17" height="13" rx="1.5" />
      <path d="M9.5 18.5v-13M14.5 18.5v-13" />
      <path d="M8 5.5v7h3v-7M13 5.5v7h3v-7" fill="currentColor" stroke="none" />
    </svg>
  );
}

export function SoundIcon() {
  // Mixer sliders: how it sounds, not how loud.
  return (
    <svg {...stroke}>
      <path d="M6 4v16M12 4v16M18 4v16" />
      <path d="M4 14h4M10 8h4M16 16h4" strokeWidth={2.4} />
    </svg>
  );
}

export function FolderIcon() {
  return (
    <svg {...stroke}>
      <path d="M3.5 7.5A2 2 0 0 1 5.5 5.5h4l2 2h7a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z" />
    </svg>
  );
}

export function ExportIcon() {
  // Out of the box: the share arrow.
  return (
    <svg {...stroke}>
      <path d="M12 14.5V4M8 7.5L12 3.5l4 4" />
      <path d="M8.5 10.5H7a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-6a2 2 0 0 0-2-2h-1.5" />
    </svg>
  );
}

function LayersIcon() {
  return (
    <svg {...stroke} width={15} height={15}>
      <path d="M12 4l8.5 4.5L12 13 3.5 8.5 12 4z" />
      <path d="M3.5 12.5L12 17l8.5-4.5" />
    </svg>
  );
}

function ChevronIcon() {
  return (
    <svg {...stroke} width={11} height={11} strokeWidth={2}>
      <path d="M6 9.5l6 6 6-6" />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg {...stroke} width={11} height={11} strokeWidth={2.6}>
      <path d="M5 12.5l4.5 4.5L19 7.5" />
    </svg>
  );
}
