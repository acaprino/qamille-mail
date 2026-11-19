/**
 * Whether an attachment opens as an email (PEC `postacert.eml`, forwarded messages) instead of a document.
 * Keep in step with `is_message` in `app/src/domain/attached_message.rs`.
 */
export function isMessageAttachment(name: string, mime: string | null | undefined): boolean {
  const type = (mime ?? '').trim().toLowerCase();
  return type === 'message/rfc822' || type === 'message/global' || /\.eml$/i.test(name.trim());
}
