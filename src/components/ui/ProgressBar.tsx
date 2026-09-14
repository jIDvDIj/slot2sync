import "./Progress.css";

interface ProgressBarProps {
  /** Fraction from 0 to 1; `null` renders an indeterminate bar. */
  value: number | null;
  label: string;
  size?: "thin" | "regular";
}

export function ProgressBar({ value, label, size = "regular" }: ProgressBarProps) {
  const determinate = value !== null;
  const clamped = determinate ? Math.min(1, Math.max(0, value)) : 0;

  return (
    <div
      className="progress-bar"
      data-size={size}
      data-indeterminate={determinate ? undefined : true}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={determinate ? Math.round(clamped * 100) : undefined}
    >
      <div
        className="progress-bar-fill"
        style={determinate ? { transform: `scaleX(${clamped})` } : undefined}
      />
    </div>
  );
}
