/**
 * Shows or hides a side panel. It sits in the title bar at the same spot
 * whether the panel is open or not, so the hand that closed a panel finds
 * the way back without looking: the library's beside the traffic lights,
 * Claude's in the top-right corner.
 */
export default function PanelToggle({
  side,
  open,
  onToggle,
  title,
  label,
  busy = false,
}: {
  side: "left" | "right";
  open: boolean;
  onToggle: () => void;
  title: string;
  label: string;
  /** Something is happening in the panel, worth a dot while it is hidden. */
  busy?: boolean;
}) {
  return (
    <button
      className={`panel-toggle ${side}${open ? " open" : ""}`}
      onClick={onToggle}
      title={title}
      aria-label={label}
      aria-expanded={open}
    >
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.6} aria-hidden>
        <rect x="3.5" y="4.5" width="17" height="15" rx="2.5" />
        {side === "left" ? <path d="M9.5 4.5v15" /> : <path d="M14.5 4.5v15" />}
        {/* The panel's side is filled in while it is showing. */}
        {open && (
          <path
            d={side === "left" ? "M6 4.5h3.5v15H6a2.5 2.5 0 0 1-2.5-2.5V7A2.5 2.5 0 0 1 6 4.5z" : "M14.5 4.5H18A2.5 2.5 0 0 1 20.5 7v10a2.5 2.5 0 0 1-2.5 2.5h-3.5z"}
            fill="currentColor"
            fillOpacity={0.35}
            stroke="none"
          />
        )}
      </svg>
      {busy && !open && <span className="panel-toggle-dot" />}
    </button>
  );
}
