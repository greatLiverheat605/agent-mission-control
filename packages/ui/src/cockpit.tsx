import type {
  ButtonHTMLAttributes,
  HTMLAttributes,
  KeyboardEvent,
  ReactNode,
} from "react";
import "./cockpit.css";

export type CockpitTone =
  | "normal"
  | "active"
  | "warning"
  | "danger"
  | "offline"
  | "degraded"
  | "verified";

function classes(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

export type CockpitFrameProps = HTMLAttributes<HTMLDivElement> & {
  children: ReactNode;
};

export function CockpitFrame({ className, children, ...props }: CockpitFrameProps) {
  return <div className={classes("mc-cockpit-frame", className)} data-cockpit-shell {...props}>{children}</div>;
}

export type ViewportBezelProps = HTMLAttributes<HTMLElement> & {
  title: string;
  meta?: ReactNode;
  controls?: ReactNode;
  children: ReactNode;
};

export function ViewportBezel({ title, meta, controls, className, children, ...props }: ViewportBezelProps) {
  return (
    <section className={classes("mc-viewport-bezel", className)} aria-label={title} {...props}>
      <header className="mc-viewport-bezel__header">
        <span className="mc-viewport-bezel__title">{title}</span>
        {meta ? <span className="mc-viewport-bezel__meta">{meta}</span> : null}
        {controls ? <div className="mc-viewport-bezel__controls">{controls}</div> : null}
      </header>
      <div className="mc-viewport-bezel__content">{children}</div>
    </section>
  );
}

export type DockSectionProps = HTMLAttributes<HTMLElement> & {
  title: string;
  icon?: ReactNode;
  meta?: ReactNode;
  children: ReactNode;
};

export function DockSection({ title, icon, meta, className, children, ...props }: DockSectionProps) {
  return (
    <section className={classes("mc-dock-section", className)} aria-label={title} {...props}>
      <header className="mc-dock-section__header">
        {icon ? <span className="mc-dock-section__icon" aria-hidden="true">{icon}</span> : null}
        <span>{title}</span>
        {meta ? <span className="mc-dock-section__meta">{meta}</span> : null}
      </header>
      <div className="mc-dock-section__body">{children}</div>
    </section>
  );
}

export type TelemetryReadoutProps = HTMLAttributes<HTMLDivElement> & {
  label: string;
  value: ReactNode;
  detail?: ReactNode;
  tone?: CockpitTone;
};

export function TelemetryReadout({ label, value, detail, tone = "normal", className, ...props }: TelemetryReadoutProps) {
  return (
    <div className={classes("mc-telemetry", className)} data-tone={tone} data-motion="state" {...props}>
      <span className="mc-telemetry__label">{label}</span>
      <strong className="mc-telemetry__value">{value}</strong>
      {detail ? <span className="mc-telemetry__detail">{detail}</span> : null}
    </div>
  );
}

export type AlertStripProps = HTMLAttributes<HTMLDivElement> & {
  tone?: Exclude<CockpitTone, "active">;
  title: string;
  children?: ReactNode;
};

export function AlertStrip({ tone = "normal", title, children, className, role, ...props }: AlertStripProps) {
  const resolvedRole = role ?? (tone === "danger" ? "alert" : "status");
  return (
    <div className={classes("mc-alert-strip", className)} data-tone={tone} data-motion="state" role={resolvedRole} {...props}>
      <span className="mc-alert-strip__signal" aria-hidden="true" />
      <strong>{title}</strong>
      {children ? <span>{children}</span> : null}
    </div>
  );
}

export type ViewSwitcherItem<T extends string> = {
  id: T;
  label: string;
  icon?: ReactNode;
  disabled?: boolean;
};

export type ViewSwitcherProps<T extends string> = Omit<HTMLAttributes<HTMLDivElement>, "onChange"> & {
  label: string;
  items: readonly ViewSwitcherItem<T>[];
  value: T;
  onChange: (value: T) => void;
};

export function nextSwitcherIndex(
  current: number,
  direction: -1 | 1,
  disabled: readonly boolean[],
) {
  if (disabled.length === 0 || disabled.every(Boolean)) return current;
  let index = current;
  do index = (index + direction + disabled.length) % disabled.length;
  while (disabled[index]);
  return index;
}

export function ViewSwitcher<T extends string>({ label, items, value, onChange, className, ...props }: ViewSwitcherProps<T>) {
  function move(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    const direction = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1
      : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1
        : null;
    if (direction === null) return;
    event.preventDefault();
    const next = nextSwitcherIndex(index, direction, items.map((item) => Boolean(item.disabled)));
    onChange(items[next].id);
    event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>("button")[next]?.focus();
  }

  return (
    <div className={classes("mc-view-switcher", className)} role="tablist" aria-label={label} {...props}>
      {items.map((item, index) => (
        <button
          className="mc-view-switcher__item"
          data-view-id={item.id}
          disabled={item.disabled}
          key={item.id}
          onClick={() => onChange(item.id)}
          onKeyDown={(event) => move(event, index)}
          role="tab"
          type="button"
          aria-selected={item.id === value}
          tabIndex={item.id === value ? 0 : -1}
        >
          {item.icon ? <span aria-hidden="true">{item.icon}</span> : null}
          <span>{item.label}</span>
        </button>
      ))}
    </div>
  );
}

export type ResponsiveDrawerProps = HTMLAttributes<HTMLElement> & {
  id: string;
  label: string;
  side: "left" | "right";
  open: boolean;
  onOpenChange: (open: boolean) => void;
  triggerProps?: Omit<ButtonHTMLAttributes<HTMLButtonElement>, "onClick">;
  children: ReactNode;
};

export function ResponsiveDrawer({ id, label, side, open, onOpenChange, triggerProps, className, children, ...props }: ResponsiveDrawerProps) {
  return (
    <div className={classes("mc-responsive-drawer", className)} data-open={open} data-side={side}>
      <button
        {...triggerProps}
        className={classes("mc-responsive-drawer__trigger", triggerProps?.className)}
        type="button"
        aria-controls={id}
        aria-expanded={open}
        data-drawer-rail={side}
        onClick={() => onOpenChange(!open)}
      >
        {label}
      </button>
      <aside id={id} className="mc-responsive-drawer__panel" aria-label={label} data-console-drawer={side} hidden={!open} {...props}>
        {children}
      </aside>
    </div>
  );
}
