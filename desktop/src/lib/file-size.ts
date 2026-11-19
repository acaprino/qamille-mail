/** Attachment size as the reader shows it: KB (at least 1) up to 1 MB, then MB with one decimal. */
export function humanSize(size: number): string {
  return size > 1048576
    ? `${(size / 1048576).toFixed(1)} MB`
    : `${Math.max(1, Math.round(size / 1024))} KB`;
}
