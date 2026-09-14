import "./Progress.css";

interface SpinnerProps {
  size?: "small" | "regular" | "large";
  /** Announced to screen readers; omit when nearby text already says what is loading. */
  label?: string;
}

export function Spinner({ size = "regular", label }: SpinnerProps) {
  return (
    <span
      className="spinner"
      data-size={size}
      role={label ? "status" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    />
  );
}
