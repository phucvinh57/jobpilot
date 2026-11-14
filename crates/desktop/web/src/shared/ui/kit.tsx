// The design kit: pure render. Views build from these instead of local helpers, and take every
// colour from the theme classes (`bg-surface`, `text-ink`, ...).

import clsx from "clsx";
import { Check, LoaderCircle, type LucideIcon } from "lucide-react";
import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactNode, TextareaHTMLAttributes } from "react";

import { TONE_CLASS, type Tone } from "../theme/tokens";
import { barFraction, scoreLabel, scoreTone, stepState } from "./rules";

// ------------------------------------------------------------------ text

/** A heading in Space Grotesk: `h1` is the page title (30 px), `h2` a card title (18 px). */
export function Heading({ level = "h2", className, children }: { level?: "h1" | "h2"; className?: string; children: ReactNode }) {
  const Tag = level;
  return (
    <Tag className={clsx("font-heading font-semibold text-ink", level === "h1" ? "text-h1" : "text-h2", className)}>
      {children}
    </Tag>
  );
}

/** Small secondary text (12 px, muted): a label, a hint, a timestamp. */
export function Caption({ className, children }: { className?: string; children: ReactNode }) {
  return <p className={clsx("text-small text-muted", className)}>{children}</p>;
}

/** A small upper-case heading inside a card. */
export function SectionLabel({ children }: { children: ReactNode }) {
  return <p className="text-small font-semibold uppercase tracking-wide text-muted">{children}</p>;
}

/** A spinner for work in progress. */
export function Spinner({ className }: { className?: string }) {
  return <LoaderCircle aria-hidden className={clsx("size-4 animate-spin", className)} />;
}

// ------------------------------------------------------------ containers

/** A bordered card with rounded corners. */
export function Card({ className, children, ...rest }: { className?: string; children: ReactNode; id?: string }) {
  return (
    <div {...rest} className={clsx("flex flex-col gap-2 rounded-card border border-border bg-surface p-4", className)}>
      {children}
    </div>
  );
}

/** A card with a title, a hint line and optional actions at the right of the title (the frame
 *  of every settings section). */
export function Section({
  title,
  hint,
  actions,
  id,
  children,
}: {
  title: string;
  hint?: string;
  actions?: ReactNode;
  id?: string;
  children?: ReactNode;
}) {
  return (
    <section id={id} aria-label={title} className="flex flex-col gap-2 rounded-card border border-border bg-surface p-4">
      <div className="flex items-center justify-between gap-2">
        <Heading>{title}</Heading>
        {actions}
      </div>
      {hint && <Caption>{hint}</Caption>}
      {children}
    </section>
  );
}

/** A highlighted strip for something that needs attention: a tone dot, a bold title, a body
 *  line, and actions on the right. */
export function Banner({
  tone,
  title,
  body,
  actions,
  id,
}: {
  tone: Tone;
  title: string;
  body?: ReactNode;
  actions?: ReactNode;
  id?: string;
}) {
  const c = TONE_CLASS[tone];
  return (
    <div id={id} role="group" aria-label={title} className={clsx("flex items-center gap-3 rounded-banner border p-3", c.border, c.bg)}>
      <span className={clsx("size-2 shrink-0 rounded-full", c.dot)} />
      <div className={clsx("flex min-w-0 flex-1 flex-col gap-0.5", c.fg)}>
        <p className="text-body font-semibold">{title}</p>
        {body && <div className="selectable text-small">{body}</div>}
      </div>
      {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
    </div>
  );
}

/** What a list shows when it has nothing: an icon, a title, a hint and an optional action. */
export function EmptyState({ icon: Icon, title, body, action }: { icon: LucideIcon; title: string; body: string; action?: ReactNode }) {
  return (
    <div className="flex flex-col items-center gap-2 rounded-card border border-dashed border-border p-6 text-center">
      <span className="flex size-11 items-center justify-center rounded-full bg-track">
        <Icon aria-hidden className="size-5 text-muted" />
      </span>
      <p className="text-body font-semibold text-ink">{title}</p>
      <p className="text-small text-muted">{body}</p>
      {action && <div className="pt-1">{action}</div>}
    </div>
  );
}

// --------------------------------------------------------------- buttons

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";

export interface ButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> {
  /** `primary` is the ONE main action of a screen; everything else is `secondary` or `ghost`. */
  variant?: ButtonVariant;
  /** `compact` for buttons inside rows and banners. */
  size?: "regular" | "compact";
  icon?: LucideIcon;
  /** Work is running: shows a spinner and disables the button. */
  busy?: boolean;
  children?: ReactNode;
}

const BUTTON_VARIANT: Record<ButtonVariant, string> = {
  primary: "border-accent bg-accent text-on-fill hover:bg-accent-dark hover:border-accent-dark",
  secondary: "border-control-border bg-surface text-ink hover:bg-track",
  ghost: "border-transparent bg-transparent text-muted hover:bg-track hover:text-ink",
  danger: "border-danger/50 bg-surface text-danger hover:bg-danger-bg",
};

export function Button({ variant = "secondary", size = "regular", icon: Icon, busy = false, disabled, className, children, ...rest }: ButtonProps) {
  return (
    <button
      type="button"
      {...rest}
      disabled={disabled || busy}
      aria-busy={busy || undefined}
      className={clsx(
        "inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 border font-medium whitespace-nowrap transition-colors",
        "disabled:cursor-not-allowed disabled:opacity-55",
        size === "regular" ? "min-h-10 rounded-button px-4 text-body" : "min-h-8 rounded-chip px-3 text-small",
        BUTTON_VARIANT[variant],
        className,
      )}
    >
      {busy ? <Spinner /> : Icon && <Icon aria-hidden className="size-4" />}
      {children}
    </button>
  );
}

/** A text that opens something: looks like a link, is a button (the webview never navigates). */
export function LinkButton({ className, children, ...rest }: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      {...rest}
      className={clsx("cursor-pointer truncate text-left text-small text-accent underline hover:text-accent-dark", className)}
    >
      {children}
    </button>
  );
}

// ---------------------------------------------------------- chips, tags

/** A small static label for a requirement the profile matches (`match`) or does not cover yet
 *  (`gap`, dashed outline). */
export function Chip({ kind, children }: { kind: "match" | "gap"; children: ReactNode }) {
  return (
    <span
      className={clsx(
        "inline-flex min-h-7 items-center rounded-chip border px-2.5 text-small font-medium",
        kind === "match" ? "border-accent-soft bg-accent-soft text-accent-dark" : "border-dashed border-bar-grey text-muted",
      )}
    >
      {children}
    </span>
  );
}

/** A filter toggle: a real button (`aria-pressed`), tinted when selected. */
export function FilterChip({ selected, className, children, ...rest }: ButtonHTMLAttributes<HTMLButtonElement> & { selected: boolean }) {
  return (
    <button
      type="button"
      {...rest}
      aria-pressed={selected}
      className={clsx(
        "inline-flex min-h-8 cursor-pointer items-center gap-1.5 rounded-chip border px-3 text-small font-medium transition-colors",
        "disabled:cursor-not-allowed disabled:opacity-55",
        selected ? "border-accent-soft bg-accent-soft text-accent-dark" : "border-control-border bg-surface text-ink hover:bg-track",
        className,
      )}
    >
      {children}
    </button>
  );
}

/** A status tag: a small rounded pill in a tone's colours. */
export function Tag({ tone, children }: { tone: Tone; children: ReactNode }) {
  const c = TONE_CLASS[tone];
  return <span className={clsx("inline-flex shrink-0 items-center rounded-tag px-2 py-0.5 text-small font-semibold", c.fg, c.bg)}>{children}</span>;
}

/** A small citation chip (`3`) that points at a stored source. */
export function CiteChip({ number, onClick }: { number: number; onClick?: () => void }) {
  return (
    <button
      type="button"
      aria-label={`Source ${number}`}
      onClick={onClick}
      className="inline-flex h-5 min-w-[22px] cursor-pointer items-center justify-center rounded-tag bg-accent-soft px-1.5 text-tiny font-semibold text-accent-dark hover:bg-accent/25"
    >
      {number}
    </button>
  );
}

/** A small status dot: green when `ok`, amber when something needs a look, grey while checking. */
export function HealthDot({ state }: { state: "ok" | "warn" | "checking" }) {
  return (
    <span
      className={clsx("size-2 shrink-0 rounded-full", state === "ok" ? "bg-ok" : state === "warn" ? "bg-warn" : "bg-bar-grey")}
    />
  );
}

// ----------------------------------------------------------------- forms

/** A form field: a label above its input. */
export function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-small font-medium text-muted">{label}</span>
      {children}
      {hint && <span className="text-small text-subtle">{hint}</span>}
    </label>
  );
}

const CONTROL =
  "w-full rounded-button border border-control-border bg-surface px-3 text-body text-ink placeholder:text-subtle " +
  "focus:border-accent focus:outline-none disabled:cursor-not-allowed disabled:opacity-55";

export function TextInput({ className, ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return <input type="text" spellCheck={false} autoComplete="off" {...rest} className={clsx(CONTROL, "min-h-10", className)} />;
}

export function TextArea({ className, ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea spellCheck={false} {...rest} className={clsx(CONTROL, "min-h-20 resize-y py-2", className)} />;
}

/** A checkbox with its label. */
export function Checkbox({ label, className, ...rest }: Omit<InputHTMLAttributes<HTMLInputElement>, "type"> & { label: ReactNode }) {
  return (
    <label className={clsx("inline-flex cursor-pointer items-center gap-2 text-body text-ink", className)}>
      <input type="checkbox" {...rest} className="size-4 cursor-pointer accent-accent" />
      {label}
    </label>
  );
}

/** A row of exclusive choices: one compact button per choice, the current one tinted. `label`
 *  names the group ("Job type: Remote", not just "Remote"). */
export function Segmented<T extends string>({
  label,
  choices,
  value,
  onChange,
  disabled,
}: {
  label: string;
  choices: readonly { value: T; label: string }[];
  value: T | undefined;
  onChange: (value: T) => void;
  disabled?: boolean;
}) {
  return (
    <div role="group" aria-label={label} className="flex flex-wrap gap-1">
      {choices.map((choice) => (
        <FilterChip
          key={choice.value}
          selected={choice.value === value}
          aria-label={`${label}: ${choice.label}`}
          disabled={disabled}
          onClick={() => onChange(choice.value)}
        >
          {choice.label}
        </FilterChip>
      ))}
    </div>
  );
}

/** Tabs: a row of buttons, the current one underlined. */
export function Tabs<T extends string>({
  label,
  tabs,
  value,
  onChange,
}: {
  label: string;
  tabs: readonly { value: T; label: string }[];
  value: T;
  onChange: (value: T) => void;
}) {
  return (
    <div role="tablist" aria-label={label} className="flex gap-1 border-b border-border">
      {tabs.map((tab) => {
        const selected = tab.value === value;
        return (
          <button
            key={tab.value}
            type="button"
            role="tab"
            aria-selected={selected}
            onClick={() => onChange(tab.value)}
            className={clsx(
              "-mb-px cursor-pointer border-b-2 px-3 py-2 text-body transition-colors",
              selected ? "border-accent font-semibold text-accent-dark" : "border-transparent text-muted hover:text-ink",
            )}
          >
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------- data display

function Bar({ fraction, colorClass }: { fraction: number; colorClass: string }) {
  return (
    <span className="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-track">
      <span className={clsx("block h-full rounded-full", colorClass)} style={{ width: `${fraction * 100}%` }} />
    </span>
  );
}

/** A square badge with a Lead's score in its tone colours (a dash when it was never scored). */
export function ScoreBadge({ score }: { score: number | null | undefined }) {
  const c = TONE_CLASS[scoreTone(score)];
  return (
    <span
      aria-label={score === null || score === undefined ? "Not scored" : `Score ${scoreLabel(score)}`}
      className={clsx("flex size-11 shrink-0 items-center justify-center rounded-button font-heading text-h2 font-semibold", c.fg, c.bg)}
    >
      {scoreLabel(score)}
    </span>
  );
}

/** One fit criterion: a label, a bar in the tone of its score (0..=100) and the number. */
export function FitBar({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex items-center gap-3">
      <span className="w-[120px] shrink-0 truncate text-small text-muted">{label}</span>
      <Bar fraction={barFraction(value, 100)} colorClass={TONE_CLASS[scoreTone(value)].dot} />
      <span className="w-7 shrink-0 text-small font-semibold text-ink">{value.toFixed(0)}</span>
    </div>
  );
}

/** A skill row of the trends chart: name, a bar scaled to the most frequent skill (accent when
 *  the skill is already in the profile, grey when it is not) and its count. */
export function SkillBar({ name, count, max, inProfile }: { name: string; count: number; max: number; inProfile: boolean }) {
  return (
    <div className="flex items-center gap-3">
      <span className="w-[120px] shrink-0 truncate text-small text-ink">{name}</span>
      <Bar fraction={barFraction(count, max)} colorClass={inProfile ? "bg-accent" : "bg-bar-grey"} />
      <span className="w-7 shrink-0 text-small text-muted">{count}</span>
    </div>
  );
}

/** A row of numbered steps joined by lines: done steps are filled, the current one is outlined. */
export function Stepper({ steps, current }: { steps: readonly string[]; current: number }) {
  return (
    <ol aria-label="Steps" className="flex items-center gap-2">
      {steps.map((label, i) => {
        const state = stepState(i, current);
        return (
          <li key={label} aria-label={`Step ${i + 1}: ${label}`} aria-current={state === "current" ? "step" : undefined} className="contents">
            <span className="flex items-center gap-2">
              <span
                className={clsx(
                  "flex size-6 shrink-0 items-center justify-center rounded-full border text-small font-semibold",
                  state === "done" && "border-accent bg-accent text-on-fill",
                  state === "current" && "border-accent bg-accent-soft text-accent-dark",
                  state === "todo" && "border-border bg-surface text-muted",
                )}
              >
                {state === "done" ? <Check aria-hidden className="size-3" /> : i + 1}
              </span>
              <span className={clsx("text-small", state === "current" && "font-semibold", state === "todo" ? "text-muted" : "text-ink")}>
                {label}
              </span>
            </span>
            {i < steps.length - 1 && <span className={clsx("h-0.5 min-w-4 flex-1 rounded-full", i < current ? "bg-accent" : "bg-border")} />}
          </li>
        );
      })}
    </ol>
  );
}

/** A card on the pipeline board: title, subtitle and a tag with the next action. */
export function KanbanCard({
  title,
  subtitle,
  next,
  onClick,
}: {
  title: string;
  subtitle: string;
  next?: { label: string; tone: Tone };
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={`${title}, ${subtitle}`}
      onClick={onClick}
      className="flex cursor-pointer flex-col gap-1.5 rounded-banner border border-border bg-surface p-3 text-left transition-colors hover:border-accent/50"
    >
      <span className="text-body font-semibold text-ink">{title}</span>
      <span className="text-small text-muted">{subtitle}</span>
      {next && (
        <span className="flex pt-1">
          <Tag tone={next.tone}>{next.label}</Tag>
        </span>
      )}
    </button>
  );
}

/** One line of the activity feed: a tone dot, what happened, and when. */
export function ActivityRow({ tone, text, when }: { tone: Tone; text: string; when: string }) {
  return (
    <li className="flex items-center gap-3 py-1.5">
      <span className={clsx("size-2 shrink-0 rounded-full", TONE_CLASS[tone].dot)} />
      <span className="min-w-0 flex-1 truncate text-body text-ink">{text}</span>
      <span className="shrink-0 text-small text-subtle">{when}</span>
    </li>
  );
}

/** The frame of a screen: a scrolling column with the page title, an optional subtitle and
 *  actions at the right. */
export function Page({
  title,
  subtitle,
  actions,
  label,
  children,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
  /** The accessible name of the screen. */
  label: string;
  children: ReactNode;
}) {
  return (
    <main aria-label={label} className="fade-in flex h-full min-w-0 flex-col gap-4 overflow-y-auto p-6">
      <header className="flex items-start justify-between gap-4">
        <div className="flex min-w-0 flex-col gap-1">
          {typeof title === "string" ? <Heading level="h1">{title}</Heading> : title}
          {subtitle && <div className="text-body text-muted">{subtitle}</div>}
        </div>
        {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
      </header>
      {children}
    </main>
  );
}
