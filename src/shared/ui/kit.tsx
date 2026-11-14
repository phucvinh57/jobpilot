// The design kit: pure render, built on the shadcn/ui primitives (`./primitives`, square: no
// rounded corners). Views build from these instead of local helpers or the primitives, and take
// every colour from the theme classes (`bg-surface`, `text-ink`, ...).

import { cva } from "class-variance-authority";
import { Check, LoaderCircle, X, type LucideIcon } from "lucide-react";
import {
  useRef,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
  type TextareaHTMLAttributes,
} from "react";

import { TONE_CLASS, type Tone } from "../theme/tokens";
import { cn } from "./cn";
import { Badge } from "./primitives/badge";
import { Button as ButtonPrimitive } from "./primitives/button";
import { Card as CardPrimitive } from "./primitives/card";
import { Input } from "./primitives/input";
import { Sheet, SheetContent, SheetTitle } from "./primitives/sheet";
import { Switch as SwitchPrimitive } from "./primitives/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "./primitives/table";
import { Textarea } from "./primitives/textarea";
import { barFraction, scoreLabel, scoreTone, stepState } from "./rules";

// ------------------------------------------------------------------ text

/** A heading in Space Grotesk: `h1` is the page title (24 px), `h2` a card title (16 px). */
export function Heading({ level = "h2", className, children }: { level?: "h1" | "h2"; className?: string; children: ReactNode }) {
  const Tag = level;
  return (
    <Tag className={cn("font-heading font-semibold text-ink", level === "h1" ? "text-h1 tracking-tight" : "text-h2", className)}>
      {children}
    </Tag>
  );
}

/** Small secondary text (12 px, muted): a label, a hint, a timestamp. */
export function Caption({ className, children }: { className?: string; children: ReactNode }) {
  return <p className={cn("text-small text-muted", className)}>{children}</p>;
}

/** A small upper-case heading inside a card. */
export function SectionLabel({ children }: { children: ReactNode }) {
  return <p className="text-tiny font-semibold tracking-wider text-muted uppercase">{children}</p>;
}

/** A spinner for work in progress. */
export function Spinner({ className }: { className?: string }) {
  return <LoaderCircle aria-hidden className={cn("size-4 animate-spin motion-reduce:animate-none", className)} />;
}

// ------------------------------------------------------------ containers

const CARD = "flex flex-col gap-2 border border-border bg-surface p-4 text-ink shadow-none";

/** A bordered card. */
export function Card({ className, children, ...rest }: { className?: string; children: ReactNode; id?: string }) {
  return (
    <CardPrimitive {...rest} className={cn(CARD, className)}>
      {children}
    </CardPrimitive>
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
    <section id={id} aria-label={title} data-slot="card" className={CARD}>
      <div className="flex items-center justify-between gap-2">
        <Heading>{title}</Heading>
        {actions}
      </div>
      {hint && <Caption>{hint}</Caption>}
      {children}
    </section>
  );
}

/** A highlighted strip for something that needs attention: a tone mark, a bold title, a body
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
    <div id={id} role="group" aria-label={title} className={cn("flex items-center gap-3 border border-l-2 px-3 py-2.5", c.border, c.bg)}>
      <span className={cn("size-2 shrink-0", c.dot)} />
      <div className={cn("flex min-w-0 flex-1 flex-col gap-0.5", c.fg)}>
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
    <div className="flex flex-col items-center gap-2 border border-dashed border-border p-6 text-center">
      <span className="flex size-10 items-center justify-center border border-border bg-track">
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

/** The kit's button looks. `primary` keeps `bg-accent`: tests find THE primary button by it. A
 *  disabled button still takes the pointer (over the primitive's `disabled:pointer-events-none`), so
 *  its `title` says why it is off; the browser never clicks a disabled button. */
const buttonLook = cva("cursor-pointer border font-medium transition-colors disabled:pointer-events-auto disabled:cursor-not-allowed disabled:opacity-55", {
  variants: {
    variant: {
      primary: "border-accent bg-accent text-on-fill enabled:hover:border-accent-dark enabled:hover:bg-accent-dark",
      secondary: "border-control-border bg-surface text-ink enabled:hover:bg-track",
      ghost: "border-transparent bg-transparent text-muted enabled:hover:bg-track enabled:hover:text-ink",
      danger: "border-danger/50 bg-surface text-danger enabled:hover:bg-danger-bg",
    },
    size: {
      regular: "h-9 px-4 text-body",
      compact: "h-8 gap-1.5 px-3 text-small",
    },
  },
});

export function Button({ variant = "secondary", size = "regular", icon: Icon, busy = false, disabled, className, children, ...rest }: ButtonProps) {
  return (
    <ButtonPrimitive
      type="button"
      {...rest}
      variant={null}
      size={null}
      disabled={disabled || busy}
      aria-busy={busy || undefined}
      className={cn(buttonLook({ variant, size }), className)}
    >
      {busy ? <Spinner /> : Icon && <Icon aria-hidden className="size-4" />}
      {children}
    </ButtonPrimitive>
  );
}

/** A text that opens something: looks like a link, is a button (the webview never navigates). */
export function LinkButton({ className, children, ...rest }: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      {...rest}
      className={cn("cursor-pointer truncate text-left text-small text-accent underline-offset-2 hover:text-accent-dark hover:underline", className)}
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
    <Badge
      variant={null}
      className={cn(
        "h-7 border px-2.5 text-small font-medium",
        kind === "match" ? "border-accent/30 bg-accent-soft text-accent-dark" : "border-dashed border-bar-grey text-muted",
      )}
    >
      {children}
    </Badge>
  );
}

/** A filter toggle: a real button (`aria-pressed`), tinted when selected. */
export function FilterChip({ selected, className, children, ...rest }: ButtonHTMLAttributes<HTMLButtonElement> & { selected: boolean }) {
  return (
    <button
      type="button"
      {...rest}
      aria-pressed={selected}
      className={cn(
        "inline-flex h-8 cursor-pointer items-center gap-1.5 border px-3 text-small font-medium transition-colors",
        "disabled:cursor-not-allowed disabled:opacity-55",
        selected ? "border-accent bg-accent-soft text-accent-dark" : "border-control-border bg-surface text-ink hover:bg-track",
        className,
      )}
    >
      {children}
    </button>
  );
}

/** A status tag: a small square label in a tone's colours. */
export function Tag({ tone, children }: { tone: Tone; children: ReactNode }) {
  const c = TONE_CLASS[tone];
  return (
    <Badge variant={null} className={cn("px-2 py-0.5 text-small font-semibold", c.fg, c.bg)}>
      {children}
    </Badge>
  );
}

/** A small citation chip (`3`) that points at a stored source. */
export function CiteChip({ number, onClick }: { number: number; onClick?: () => void }) {
  return (
    <button
      type="button"
      aria-label={`Source ${number}`}
      onClick={onClick}
      className="inline-flex h-5 min-w-[22px] cursor-pointer items-center justify-center bg-accent-soft px-1.5 text-tiny font-semibold text-accent-dark hover:bg-accent/25"
    >
      {number}
    </button>
  );
}

/** A small status dot: green when `ok`, amber when something needs a look, grey while checking.
 *  (The one round shape of the app.) */
export function HealthDot({ state }: { state: "ok" | "warn" | "checking" }) {
  return <span className={cn("size-2 shrink-0 rounded-full", state === "ok" ? "bg-ok" : state === "warn" ? "bg-warn" : "bg-bar-grey")} />;
}

// ----------------------------------------------------------------- forms

/** A form field: a label above its input. */
export function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-small font-medium text-muted">{label}</span>
      {children}
      {hint && <span className="text-small text-subtle">{hint}</span>}
    </label>
  );
}

const CONTROL =
  "border-control-border bg-surface text-ink shadow-none placeholder:text-subtle focus-visible:border-accent focus-visible:ring-2 " +
  "focus-visible:ring-accent/25 disabled:opacity-55 dark:bg-surface";

export function TextInput({ className, ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return <Input type="text" spellCheck={false} autoComplete="off" {...rest} className={cn(CONTROL, "h-9", className)} />;
}

export function TextArea({ className, ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <Textarea spellCheck={false} {...rest} className={cn(CONTROL, "min-h-20 resize-y", className)} />;
}

/** A checkbox with its label: a native input (so `onChange` gets `e.target.checked`), drawn as a
 *  square box with a check mark. */
export function Checkbox({ label, className, ...rest }: Omit<InputHTMLAttributes<HTMLInputElement>, "type"> & { label: ReactNode }) {
  return (
    <label className={cn("inline-flex cursor-pointer items-center gap-2 text-body text-ink has-disabled:cursor-not-allowed has-disabled:opacity-55", className)}>
      <span className="relative inline-grid size-4 shrink-0 place-items-center">
        <input
          type="checkbox"
          {...rest}
          className={cn(
            "peer col-start-1 row-start-1 size-4 cursor-pointer appearance-none border border-control-border bg-surface transition-colors",
            "checked:border-accent checked:bg-accent disabled:cursor-not-allowed",
          )}
        />
        <Check aria-hidden strokeWidth={3} className="pointer-events-none col-start-1 row-start-1 size-3 text-on-fill opacity-0 peer-checked:opacity-100" />
      </span>
      {label}
    </label>
  );
}

/** An on/off switch: a real button with `role="switch"`, so it is keyboard-operable and read as
 *  "on" / "off". `label` is the visible text next to it; without one, give `ariaLabel`. */
export function Switch({
  id,
  checked,
  onChange,
  label,
  ariaLabel,
  disabled,
}: {
  id?: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  label?: ReactNode;
  ariaLabel?: string;
  disabled?: boolean;
}) {
  return (
    <label className={cn("inline-flex items-center gap-3 text-body text-ink", disabled ? "cursor-not-allowed opacity-55" : "cursor-pointer")}>
      <SwitchPrimitive
        id={id}
        checked={checked}
        onCheckedChange={onChange}
        aria-label={ariaLabel}
        disabled={disabled}
        className="cursor-pointer"
      />
      {label}
    </label>
  );
}

/** A row of exclusive choices: joined buttons, the current one tinted. `label` names the group
 *  ("Job type: Remote", not just "Remote"). */
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
    <div role="group" aria-label={label} className="flex flex-wrap">
      {choices.map((choice) => (
        <FilterChip
          key={choice.value}
          selected={choice.value === value}
          aria-label={`${label}: ${choice.label}`}
          disabled={disabled}
          onClick={() => onChange(choice.value)}
          className="-ml-px first:ml-0 focus-visible:z-10 aria-pressed:z-[1]"
        >
          {choice.label}
        </FilterChip>
      ))}
    </div>
  );
}

/** Arrow keys move between the tabs of a tab strip (and select the one they land on). */
function onTabKeys<T>(event: KeyboardEvent<HTMLElement>, keys: readonly T[], value: T, onChange: (value: T) => void) {
  const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
  const at = keys.indexOf(value);
  if (step === 0 || at < 0 || keys.length === 0) {
    return;
  }
  event.preventDefault();
  const next = keys[(at + step + keys.length) % keys.length];
  if (next !== undefined) {
    onChange(next);
    const tabs = event.currentTarget.querySelectorAll<HTMLElement>('[role="tab"]');
    tabs[keys.indexOf(next)]?.focus();
  }
}

/** Tabs: a row of buttons, the current one underlined in the accent. */
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
  const keys = tabs.map((tab) => tab.value);
  return (
    <div role="tablist" aria-label={label} onKeyDown={(e) => onTabKeys(e, keys, value, onChange)} className="flex gap-1 border-b border-border">
      {tabs.map((tab) => {
        const selected = tab.value === value;
        return (
          <button
            key={tab.value}
            type="button"
            role="tab"
            aria-selected={selected}
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(tab.value)}
            className={cn(
              "-mb-px cursor-pointer border-b-2 px-3 py-2 text-body font-medium transition-colors",
              selected ? "border-accent text-ink" : "border-transparent text-muted hover:text-ink",
            )}
          >
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}

/** A strip of pipeline stages (tabs with a count): the current one is marked by an accent bar.
 *  Each tab has the id `stage-<key>` and is named `<label>, <count>`. */
export function StageBar<T extends string>({
  label,
  stages,
  value,
  onChange,
}: {
  label: string;
  stages: readonly { key: T; label: string; count: number }[];
  value: T;
  onChange: (value: T) => void;
}) {
  const keys = stages.map((stage) => stage.key);
  return (
    <div role="tablist" aria-label={label} onKeyDown={(e) => onTabKeys(e, keys, value, onChange)} className="flex flex-wrap border border-border bg-surface">
      {stages.map((stage) => {
        const selected = stage.key === value;
        return (
          <button
            key={stage.key}
            id={`stage-${stage.key}`}
            type="button"
            role="tab"
            aria-label={`${stage.label}, ${stage.count}`}
            aria-selected={selected}
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(stage.key)}
            className={cn(
              "-mb-px flex min-w-0 cursor-pointer items-center gap-2 border-b-2 px-3 py-2 text-body transition-colors not-last:border-r not-last:border-r-border",
              selected ? "border-b-accent bg-track font-semibold text-ink" : "border-b-transparent text-muted hover:bg-track hover:text-ink",
            )}
          >
            <span className="truncate">{stage.label}</span>
            <span className={cn("shrink-0 px-1.5 text-tiny font-semibold", selected ? "bg-accent-soft text-accent-dark" : "bg-track text-muted")}>{stage.count}</span>
          </button>
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------- data display

/** A thin square bar (the look of the shadcn Progress). */
function Bar({ fraction, colorClass }: { fraction: number; colorClass: string }) {
  return (
    <span className="h-1.5 min-w-0 flex-1 overflow-hidden bg-track">
      <span className={cn("block h-full transition-[width]", colorClass)} style={{ width: `${fraction * 100}%` }} />
    </span>
  );
}

/** A square badge with a Lead's score in its tone colours (a dash when it was never scored). */
export function ScoreBadge({ score }: { score: number | null | undefined }) {
  const c = TONE_CLASS[scoreTone(score)];
  return (
    <span
      aria-label={score === null || score === undefined ? "Not scored" : `Score ${scoreLabel(score)}`}
      className={cn("flex size-10 shrink-0 items-center justify-center font-heading text-h2 font-semibold", c.fg, c.bg)}
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
      <span className="w-7 shrink-0 text-small font-semibold text-ink tabular-nums">{value.toFixed(0)}</span>
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
      <span className="w-7 shrink-0 text-small text-muted tabular-nums">{count}</span>
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
                className={cn(
                  "flex size-6 shrink-0 items-center justify-center border text-small font-semibold",
                  state === "done" && "border-accent bg-accent text-on-fill",
                  state === "current" && "border-accent bg-accent-soft text-accent-dark",
                  state === "todo" && "border-border bg-surface text-muted",
                )}
              >
                {state === "done" ? <Check aria-hidden className="size-3" /> : i + 1}
              </span>
              <span className={cn("text-small", state === "current" && "font-semibold", state === "todo" ? "text-muted" : "text-ink")}>
                {label}
              </span>
            </span>
            {i < steps.length - 1 && <span className={cn("h-px min-w-4 flex-1", i < current ? "bg-accent" : "bg-border")} />}
          </li>
        );
      })}
    </ol>
  );
}

/** One line of the activity feed: a tone mark, what happened, and when. */
export function ActivityRow({ tone, text, when }: { tone: Tone; text: string; when: string }) {
  return (
    <li className="flex items-center gap-3 py-1.5">
      <span className={cn("size-2 shrink-0", TONE_CLASS[tone].dot)} />
      <span className="min-w-0 flex-1 truncate text-body text-ink">{text}</span>
      <span className="shrink-0 text-small text-subtle">{when}</span>
    </li>
  );
}

export interface DataColumn<T> {
  key: string;
  header: ReactNode;
  className?: string;
  cell: (row: T) => ReactNode;
}

/** Whether an event started on a control inside a row (a checkbox, a button), which handles it
 *  itself: the row must not open then. */
function fromControl(event: MouseEvent<HTMLElement> | KeyboardEvent<HTMLElement>): boolean {
  const target = event.target as HTMLElement;
  const control = target.closest("button, a, input, select, textarea, label, [role='checkbox'], [role='switch']");
  return control !== null && control !== event.currentTarget && event.currentTarget.contains(control);
}

/** A table of rows that open something (a drawer): click a row, or focus it and press Enter or
 *  Space. The open row is `aria-current`. Controls inside a cell keep their own clicks. */
export function DataTable<T>({
  label,
  columns,
  rows,
  rowKey,
  onOpen,
  openKey,
  rowLabel,
  empty,
  id,
}: {
  label: string;
  columns: readonly DataColumn<T>[];
  rows: readonly T[];
  rowKey: (row: T) => string;
  onOpen?: (row: T) => void;
  openKey?: string | null;
  rowLabel?: (row: T) => string;
  empty?: ReactNode;
  id?: string;
}) {
  return (
    <div className="border border-border bg-surface">
      <Table id={id} aria-label={label} className="text-body text-ink">
        <TableHeader className="bg-surface-alt">
          <TableRow className="hover:bg-transparent">
            {columns.map((column) => (
              <TableHead key={column.key} className={cn("h-9 px-3 text-tiny font-semibold tracking-wider text-muted uppercase", column.className)}>
                {column.header}
              </TableHead>
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.length === 0 && (
            <TableRow className="hover:bg-transparent">
              <TableCell colSpan={columns.length} className="px-3 py-6 text-center whitespace-normal text-muted">
                {empty ?? "Nothing here."}
              </TableCell>
            </TableRow>
          )}
          {rows.map((row) => {
            const key = rowKey(row);
            const open = onOpen;
            const current = openKey !== undefined && openKey !== null && openKey === key;
            return (
              <TableRow
                key={key}
                aria-label={rowLabel?.(row)}
                aria-current={current ? "true" : undefined}
                tabIndex={open ? 0 : undefined}
                onClick={open ? (e) => !fromControl(e) && open(row) : undefined}
                onKeyDown={
                  open
                    ? (e) => {
                        if ((e.key === "Enter" || e.key === " ") && !fromControl(e)) {
                          e.preventDefault();
                          open(row);
                        }
                      }
                    : undefined
                }
                className={cn(
                  "border-border hover:bg-track focus-visible:-outline-offset-2",
                  open && "cursor-pointer",
                  current && "bg-track shadow-[inset_2px_0_0_var(--jp-accent)] hover:bg-track",
                )}
              >
                {columns.map((column) => (
                  <TableCell key={column.key} className={cn("px-3 py-2", column.className)}>
                    {column.cell(row)}
                  </TableCell>
                ))}
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}

// ---------------------------------------------------------------- frames

/** A side panel at the right: non-modal (the page behind stays usable, so pressing another row
 *  just switches what it shows), closed by Esc or its ✕. Focus moves in on open and goes back
 *  where it was on close. */
export function Drawer({
  open,
  title,
  width = "md",
  onClose,
  actions,
  children,
  id = "drawer",
}: {
  open: boolean;
  title: string;
  width?: "md" | "lg";
  onClose: () => void;
  actions?: ReactNode;
  children?: ReactNode;
  id?: string;
}) {
  // Radix gives focus back to a dialog trigger; this drawer has none, so it remembers.
  const returnTo = useRef<HTMLElement | null>(null);
  return (
    <Sheet modal={false} open={open} onOpenChange={(next) => !next && onClose()}>
      <SheetContent
        id={id}
        side="right"
        showCloseButton={false}
        aria-describedby={undefined}
        onOpenAutoFocus={() => {
          returnTo.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        }}
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          const back = returnTo.current;
          returnTo.current = null;
          if (back && back.isConnected) {
            back.focus();
          }
        }}
        onInteractOutside={(event) => event.preventDefault()}
        className={cn(
          "gap-0 border-l border-border bg-surface text-ink shadow-xl motion-reduce:animate-none!",
          "max-w-[calc(100vw-2rem)] sm:max-w-[calc(100vw-2rem)]",
          width === "lg" ? "w-[760px]" : "w-[480px]",
        )}
      >
        <header className="flex items-center gap-2 border-b border-border px-4 py-3">
          <SheetTitle asChild>
            <h2 className="min-w-0 flex-1 truncate font-heading text-h2 font-semibold text-ink">{title}</h2>
          </SheetTitle>
          {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
          <Button id="drawer-close" variant="ghost" size="compact" aria-label="Close" icon={X} onClick={onClose} className="px-2" />
        </header>
        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4">{children}</div>
      </SheetContent>
    </Sheet>
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
    <main aria-label={label} className="fade-in flex h-full min-w-0 flex-col gap-4 overflow-y-auto px-6 py-5">
      <header className="flex items-start justify-between gap-4 border-b border-border pb-4">
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
