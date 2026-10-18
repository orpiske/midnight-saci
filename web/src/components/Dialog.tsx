import { X } from "lucide-react";
import { type ReactNode, useEffect, useId, useRef } from "react";

/** A modal on the native <dialog>: focus moves in and is trapped by the
 * browser, Escape closes it, and focus returns to the opener. */
export function Dialog({
  open,
  title,
  onClose,
  children,
  footer,
  wide,
}: {
  open: boolean;
  title: ReactNode;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  wide?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const opener = useRef<Element | null>(null);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      opener.current = document.activeElement;
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open]);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    const closed = () => {
      onClose();
      if (opener.current instanceof HTMLElement) opener.current.focus();
    };
    dialog.addEventListener("close", closed);
    return () => dialog.removeEventListener("close", closed);
  }, [onClose]);

  return (
    <dialog
      ref={ref}
      className="modal"
      aria-labelledby={titleId}
      style={wide ? { width: "min(960px, calc(100vw - 32px))" } : undefined}
    >
      {open ? (
        <>
          <div className="modal-header">
            <h2 id={titleId}>{title}</h2>
            <button
              type="button"
              className="icon-btn"
              aria-label="Close"
              onClick={() => ref.current?.close()}
            >
              <X size={16} aria-hidden="true" />
            </button>
          </div>
          <div className="modal-body">{children}</div>
          {footer ? <div className="modal-footer">{footer}</div> : null}
        </>
      ) : null}
    </dialog>
  );
}
