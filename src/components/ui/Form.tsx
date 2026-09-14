import {
  useId,
  useRef,
  type ComponentPropsWithRef,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
} from "react";

import { cx } from "../../lib/cx";
import { Icon, type IconName } from "./Icon";

import "./Form.css";

interface FormSectionProps {
  title?: ReactNode;
  description?: ReactNode;
  /** Explanatory text under the group. */
  footer?: ReactNode;
  children: ReactNode;
}

/** A titled, inset group of rows: the building block of settings-like screens and lists. */
export function FormSection({ title, description, footer, children }: FormSectionProps) {
  const headingId = useId();
  return (
    <section className="form-section" aria-labelledby={title ? headingId : undefined}>
      {title || description ? (
        <div className="form-section-header">
          {title ? (
            <h2 id={headingId} className="form-section-title">
              {title}
            </h2>
          ) : null}
          {description ? <p className="form-section-description">{description}</p> : null}
        </div>
      ) : null}
      <div className="form-group">{children}</div>
      {footer ? <div className="form-section-footer">{footer}</div> : null}
    </section>
  );
}

interface FormRowProps {
  label: ReactNode;
  description?: ReactNode;
  /** Id of the control the label names. */
  htmlFor?: string;
  control?: ReactNode;
  /** Content rendered under the label/control line (inline errors, expanded detail). */
  children?: ReactNode;
  icon?: IconName;
}

export function FormRow({ label, description, htmlFor, control, children, icon }: FormRowProps) {
  return (
    <div className="form-row">
      <div className="form-row-main">
        {icon ? <Icon name={icon} size={16} className="form-row-icon" /> : null}
        <div className="form-row-text">
          {htmlFor ? (
            <label className="form-row-label" htmlFor={htmlFor}>
              {label}
            </label>
          ) : (
            <span className="form-row-label">{label}</span>
          )}
          {description ? <span className="form-row-description">{description}</span> : null}
        </div>
        {control ? <div className="form-row-control">{control}</div> : null}
      </div>
      {children ? <div className="form-row-extra">{children}</div> : null}
    </div>
  );
}

interface FieldProps {
  label: ReactNode;
  htmlFor: string;
  hint?: ReactNode;
  error?: ReactNode;
  children: ReactNode;
}

/** Stacked label + control + hint/error, for forms outside grouped rows. */
export function Field({ label, htmlFor, hint, error, children }: FieldProps) {
  return (
    <div className="field">
      <label className="field-label" htmlFor={htmlFor}>
        {label}
      </label>
      {children}
      {error ? (
        <InlineStatus tone="danger">{error}</InlineStatus>
      ) : hint ? (
        <span className="field-hint">{hint}</span>
      ) : null}
    </div>
  );
}

interface TextFieldProps extends ComponentPropsWithRef<"input"> {
  invalid?: boolean;
}

export function TextField({ className, invalid, type = "text", ...rest }: TextFieldProps) {
  return (
    <input
      type={type}
      className={cx("text-field", className)}
      aria-invalid={invalid || undefined}
      {...rest}
    />
  );
}

export function Select({ className, children, ...rest }: ComponentPropsWithRef<"select">) {
  return (
    <span className={cx("select", className)}>
      <select className="select-input" {...rest}>
        {children}
      </select>
      <Icon name="chevronUpDown" size={12} className="select-icon" />
    </span>
  );
}

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  icon?: IconName;
}

interface SegmentedControlProps<T extends string> {
  value: T;
  options: readonly SegmentedOption<T>[];
  onChange: (value: T) => void;
  "aria-label": string;
}

/** Mutually exclusive choice among a few peers; arrow keys move the selection. */
export function SegmentedControl<T extends string>({
  value,
  options,
  onChange,
  "aria-label": ariaLabel,
}: SegmentedControlProps<T>) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const index = Math.max(
    0,
    options.findIndex((option) => option.value === value),
  );

  const select = (next: number) => {
    const wrapped = (next + options.length) % options.length;
    onChange(options[wrapped].value);
    refs.current[wrapped]?.focus();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, position: number) => {
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      event.preventDefault();
      select(position + 1);
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      event.preventDefault();
      select(position - 1);
    }
  };

  return (
    <div
      className="segmented"
      role="radiogroup"
      aria-label={ariaLabel}
      style={{ "--segment-count": options.length, "--segment-index": index } as CSSProperties}
    >
      <span className="segmented-thumb" aria-hidden="true" />
      {options.map((option, position) => (
        <button
          key={option.value}
          ref={(element) => {
            refs.current[position] = element;
          }}
          type="button"
          role="radio"
          aria-checked={position === index}
          tabIndex={position === index ? 0 : -1}
          className="segmented-option"
          onClick={() => onChange(option.value)}
          onKeyDown={(event) => onKeyDown(event, position)}
        >
          {option.icon ? <Icon name={option.icon} size={14} /> : null}
          <span>{option.label}</span>
        </button>
      ))}
    </div>
  );
}

interface InlineStatusProps {
  tone?: "neutral" | "success" | "danger";
  children: ReactNode;
}

/** Feedback that lives next to the control it concerns ("Saved", a validation error). */
export function InlineStatus({ tone = "neutral", children }: InlineStatusProps) {
  const icon: IconName | null = tone === "success" ? "check" : tone === "danger" ? "error" : null;
  return (
    <span className="inline-status" data-tone={tone} role={tone === "danger" ? "alert" : "status"}>
      {icon ? <Icon name={icon} size={13} /> : null}
      <span>{children}</span>
    </span>
  );
}
