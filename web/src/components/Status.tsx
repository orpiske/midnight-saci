import type { StatusLook } from "../lib/status";

/** A status pill: tinted background, icon and label (artwork/BRAND.md 6). */
export function Badge({ look, label }: { look: StatusLook; label?: string }) {
  const Icon = look.icon;
  return (
    <span className={`badge tone-${look.tone}${look.live ? " pulse" : ""}`}>
      <Icon size={14} strokeWidth={2} aria-hidden="true" className={look.live ? "spin" : undefined} />
      <span>{label ?? look.label}</span>
    </span>
  );
}

/** An icon with its label for assistive technology only, where the label
 * also appears as visible text nearby. */
export function StatusIcon({ look, size = 16 }: { look: StatusLook; size?: number }) {
  const Icon = look.icon;
  return (
    <Icon
      size={size}
      strokeWidth={1.75}
      className={`status-icon tone-${look.tone}`}
      role="img"
      aria-label={look.label}
    />
  );
}
