/** An inline result that is not an error ("Saved.", "Works"). Errors use `ErrorBanner`. */
export function Notice({ text }: { text: string }) {
  return (
    <p role="status" className="selectable text-small text-ok">
      {text}
    </p>
  );
}
