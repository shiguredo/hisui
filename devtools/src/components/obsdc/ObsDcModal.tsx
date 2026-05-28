import type { ComponentChildren } from "preact";
import { useEffect, useRef } from "preact/hooks";

interface ObsDcModalProps {
  open: boolean;
  title: string;
  onClose: () => void;
  children: ComponentChildren;
}

export function ObsDcModal({ open, title, onClose, children }: ObsDcModalProps) {
  const contentRef = useRef<HTMLDivElement>(null);

  // Escape キーで閉じる
  useEffect(() => {
    if (!open) {
      return;
    }
    function handleKeyDown(event: KeyboardEvent): void {
      if (event.key === "Escape") {
        onClose();
      }
    }
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [open, onClose]);

  if (!open) {
    return null;
  }

  function handleOverlayClick(event: MouseEvent): void {
    if (contentRef.current && !contentRef.current.contains(event.target as Node)) {
      onClose();
    }
  }

  return (
    <div
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onClick={handleOverlayClick}
    >
      <div ref={contentRef} class="w-96 rounded-lg border border-surface-200 bg-white shadow-xl">
        <div class="flex items-center justify-between border-b border-surface-200 px-4 py-3">
          <span class="text-sm font-medium text-slate-800">{title}</span>
          <button type="button" onClick={onClose} class="text-slate-600 hover:text-slate-800">
            &times;
          </button>
        </div>
        <div class="p-4">{children}</div>
      </div>
    </div>
  );
}
