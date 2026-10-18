import { type KeyboardEvent, type ReactNode, useRef } from "react";

export interface TabItem<K extends string> {
  key: K;
  label: ReactNode;
}

/** ARIA tabs with arrow-key, Home and End navigation. The panel is
 * rendered by the caller with `tabPanelProps`. */
export function Tabs<K extends string>({
  id,
  label,
  items,
  selected,
  onSelect,
  small,
}: {
  id: string;
  label: string;
  items: readonly TabItem<K>[];
  selected: K;
  onSelect: (key: K) => void;
  small?: boolean;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const move = (event: KeyboardEvent, index: number) => {
    const last = items.length - 1;
    const next =
      event.key === "ArrowRight"
        ? index === last
          ? 0
          : index + 1
        : event.key === "ArrowLeft"
          ? index === 0
            ? last
            : index - 1
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? last
              : null;
    if (next === null) return;
    event.preventDefault();
    const item = items[next];
    if (!item) return;
    onSelect(item.key);
    refs.current[next]?.focus();
  };
  return (
    <div className={`tabs${small ? " tabs-sm" : ""}`} role="tablist" aria-label={label}>
      {items.map((item, index) => {
        const active = item.key === selected;
        return (
          <button
            key={item.key}
            ref={(element) => {
              refs.current[index] = element;
            }}
            type="button"
            role="tab"
            id={`${id}-tab-${item.key}`}
            aria-selected={active}
            aria-controls={`${id}-panel`}
            tabIndex={active ? 0 : -1}
            className="tab"
            onClick={() => onSelect(item.key)}
            onKeyDown={(event) => move(event, index)}
          >
            {item.label}
          </button>
        );
      })}
    </div>
  );
}

export function tabPanelProps(id: string, selected: string) {
  return {
    role: "tabpanel" as const,
    id: `${id}-panel`,
    "aria-labelledby": `${id}-tab-${selected}`,
    tabIndex: 0,
  };
}
