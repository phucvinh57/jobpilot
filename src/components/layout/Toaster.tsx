// The toasts of the app (`stores/toast`), bottom right. Each one leaves after its time (a failure
// with Retry stays until it is closed); the pointer or the focus on it holds it.

import { useEffect, useRef, useState } from "react";

import { ToastCard, ToastRegion } from "@/components/ui/kit";
import { ProblemActions } from "@/components/ProblemHelp";
import { toastDuration, useToasts, type Toast } from "@/stores/toast";

function ToastView({ item, onClose }: { item: Toast; onClose: () => void }) {
  const [paused, setPaused] = useState(false);
  const left = useRef(toastDuration(item));
  const close = useRef(onClose);
  close.current = onClose;

  useEffect(() => {
    if (paused || left.current === null) {
      return;
    }
    const started = Date.now();
    const timer = setTimeout(() => close.current(), left.current);
    return () => {
      clearTimeout(timer);
      if (left.current !== null) {
        left.current -= Date.now() - started;
      }
    };
  }, [paused]);

  const hold = {
    onClose,
    onPause: () => setPaused(true),
    onResume: () => setPaused(false),
  };

  if (item.kind !== "error") {
    return <ToastCard tone={item.kind === "ok" ? "ok" : "neutral"} title={item.title} body={item.body} {...hold} />;
  }
  const { error, onRetry, onOpenBrowser } = item;
  const { problem } = error;
  return (
    <ToastCard
      tone={problem.kind === "needsHuman" ? "warn" : "danger"}
      title={item.title}
      body={
        <>
          <p>{error.message}</p>
          {problem.advice && <p>{problem.advice}</p>}
        </>
      }
      actions={
        <ProblemActions
          problem={problem}
          onRetry={
            onRetry &&
            (() => {
              onClose();
              onRetry();
            })
          }
          onOpenBrowser={onOpenBrowser}
        />
      }
      {...hold}
    />
  );
}

export function Toaster() {
  const toasts = useToasts((s) => s.toasts);
  const dismiss = useToasts((s) => s.dismiss);
  return (
    <ToastRegion>
      {toasts.map((item) => (
        <ToastView
          key={item.id}
          item={item}
          onClose={() => {
            dismiss(item.id);
            if (item.kind === "error") {
              item.onDismiss?.();
            }
          }}
        />
      ))}
    </ToastRegion>
  );
}
