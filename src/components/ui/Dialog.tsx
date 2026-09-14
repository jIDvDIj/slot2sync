import {
  useEffect,
  useId,
  useRef,
  useState,
  type MouseEvent,
  type ReactNode,
  type SyntheticEvent,
} from "react";
import { useTranslation } from "react-i18next";

import { Button } from "./Button";

import "./Dialog.css";

const EXIT_MS = 180;

export interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  description?: ReactNode;
  children?: ReactNode;
  /** Action buttons, right-aligned; the primary action goes last. */
  footer?: ReactNode;
  size?: "small" | "medium" | "large";
  role?: "dialog" | "alertdialog";
  /** When false, Escape and clicks outside do nothing (use for in-flight work). */
  dismissible?: boolean;
  hideCloseButton?: boolean;
}

/**
 * Modal built on the native <dialog>: focus trapping, Escape and the top layer
 * come from the browser. Mark the element that should receive focus on open
 * with `data-autofocus`.
 */
export function Dialog({
  open,
  onClose,
  title,
  description,
  children,
  footer,
  size = "medium",
  role = "dialog",
  dismissible = true,
  hideCloseButton = false,
}: DialogProps) {
  const { t } = useTranslation();
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const descriptionId = useId();
  const [mounted, setMounted] = useState(open);
  if (open && !mounted) setMounted(true);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open) {
      dialog.dataset.state = "open";
      if (!dialog.open) {
        dialog.showModal();
        dialog.querySelector<HTMLElement>("[data-autofocus]")?.focus();
      }
      return;
    }
    if (!dialog.open) return;
    // Keep the element mounted until the exit animation finishes.
    dialog.dataset.state = "closing";
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const timer = window.setTimeout(
      () => {
        dialog.close();
        setMounted(false);
      },
      reduceMotion ? 0 : EXIT_MS,
    );
    return () => window.clearTimeout(timer);
  }, [open, mounted]);

  if (!mounted) return null;

  const handleCancel = (event: SyntheticEvent<HTMLDialogElement>) => {
    event.preventDefault();
    if (dismissible) onClose();
  };

  const handleClick = (event: MouseEvent<HTMLDialogElement>) => {
    if (!dismissible || event.target !== event.currentTarget) return;
    const rect = event.currentTarget.getBoundingClientRect();
    const inside =
      event.clientX >= rect.left &&
      event.clientX <= rect.right &&
      event.clientY >= rect.top &&
      event.clientY <= rect.bottom;
    if (!inside) onClose();
  };

  return (
    <dialog
      ref={ref}
      className="dialog"
      data-size={size}
      role={role === "alertdialog" ? "alertdialog" : undefined}
      aria-labelledby={titleId}
      aria-describedby={description ? descriptionId : undefined}
      onCancel={handleCancel}
      onClick={handleClick}
    >
      <header className="dialog-header">
        <div className="dialog-heading">
          <h2 id={titleId} className="dialog-title">
            {title}
          </h2>
          {description ? (
            <p id={descriptionId} className="dialog-description">
              {description}
            </p>
          ) : null}
        </div>
        {dismissible && !hideCloseButton ? (
          <Button
            variant="plain"
            size="small"
            icon="close"
            aria-label={t("common.close")}
            onClick={onClose}
            className="dialog-close"
          />
        ) : null}
      </header>
      {children ? <div className="dialog-body">{children}</div> : null}
      {footer ? <footer className="dialog-footer">{footer}</footer> : null}
    </dialog>
  );
}

interface ConfirmDialogProps {
  open: boolean;
  title: ReactNode;
  message: ReactNode;
  confirmLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
  destructive?: boolean;
  busy?: boolean;
}

/** Short alert for a consequential choice; Cancel takes focus when the action is destructive. */
export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel,
  onConfirm,
  onCancel,
  destructive = false,
  busy = false,
}: ConfirmDialogProps) {
  const { t } = useTranslation();
  return (
    <Dialog
      open={open}
      onClose={onCancel}
      title={title}
      description={message}
      size="small"
      role="alertdialog"
      dismissible={!busy}
      hideCloseButton
      footer={
        <>
          <Button
            variant="bordered"
            onClick={onCancel}
            disabled={busy}
            data-autofocus={destructive || undefined}
          >
            {t("common.cancel")}
          </Button>
          <Button
            variant={destructive ? "destructiveProminent" : "prominent"}
            onClick={onConfirm}
            loading={busy}
            data-autofocus={destructive ? undefined : true}
          >
            {confirmLabel}
          </Button>
        </>
      }
    />
  );
}
