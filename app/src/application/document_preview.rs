//! Read-only visual previews for mail attachments. The pages are rendered in the
//! WebView by the embedded ArtCraft WebAssembly engines; this service only hands over bytes.
use super::{AppError, AppResult, document_preview_ports::visual_extension, mail::MailService};

#[derive(Clone)]
pub struct DocumentPreviewService {
    mail: MailService,
}

impl DocumentPreviewService {
    pub fn new(mail: MailService) -> Self {
        Self { mail }
    }
    /// Name and raw bytes of any attachment, or of a document nested inside an attached message,
    /// for saving it to disk.
    pub fn file_to_save(
        &self,
        email_id: u64,
        attachment_id: u64,
        path: &[u32],
    ) -> AppResult<(String, Vec<u8>)> {
        let attachment = self.mail.attached_document(email_id, attachment_id, path)?;
        Ok((attachment.name, attachment.bytes))
    }
    /// Kind (`pdf`, `docx`, `xlsx`, `pptx`) and raw bytes of an attachment (`path` empty) or of a
    /// document nested inside an attached message (see `domain::attached_message`). Other types
    /// answer `preview_unsupported`.
    pub fn document_source(
        &self,
        email_id: u64,
        attachment_id: u64,
        path: &[u32],
    ) -> AppResult<(&'static str, Vec<u8>)> {
        let attachment = self.mail.attached_document(email_id, attachment_id, path)?;
        let Some(kind) = visual_extension(&attachment.name) else {
            return Err(AppError::new(
                "preview_unsupported",
                "No visual preview is available for this file type.",
            ));
        };
        if attachment.bytes.len() as u64 > crate::rules::ATTACHMENT_CAP_BYTES {
            return Err(AppError::validation("File is larger than 25 MB."));
        }
        Ok((kind, attachment.bytes))
    }
}
