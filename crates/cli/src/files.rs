//! Attachments: files a Device sends with a message. The bytes leave as a `file` blob under
//! the attachment's id, encrypted with the account key like a chat op, and land in
//! `~/.lorca/files/<id>` on every Device that needs them: the Runner copies them into the
//! bot's working directory for its turn, and the app shows them in the transcript.

use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(feature = "runner")]
use base64::Engine;
#[cfg(feature = "runner")]
use lorca_agent::ContentPart;

use crate::app::App;
use crate::model::Attachment;

pub const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;
/// Images up to this size go to the model as pixels as well as a path.
#[cfg(feature = "runner")]
const MAX_IMAGE_PART_BYTES: u64 = 5 * 1024 * 1024;

/// A file the app asked to send: a path on this machine, with the id the app already shows.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct OutgoingFile {
    pub path: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// The app's own guesses, kept when given so the bubble it already shows does not change.
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

pub fn new_id() -> String {
    let mut bytes = [0u8; 6];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    format!("att-{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

pub fn local_path(app: &App, id: &str) -> PathBuf {
    app.config.files_dir().join(id)
}

pub fn is_local(app: &App, id: &str) -> bool {
    local_path(app, id).is_file()
}

/// Copies a file into the store and returns its attachment record. The bytes are not uploaded
/// here; `push_blob` does that once the message is ready.
pub fn store(app: &App, file: &OutgoingFile) -> anyhow::Result<Attachment> {
    let source = Path::new(&file.path);
    let metadata = std::fs::metadata(source)?;
    if !metadata.is_file() {
        anyhow::bail!("{} is not a file", file.path);
    }
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        anyhow::bail!("{} is larger than {} MB", file.path, MAX_ATTACHMENT_BYTES / 1024 / 1024);
    }
    let id = match &file.id {
        Some(id) if id.starts_with("att-") && id.len() <= 48 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => id.clone(),
        _ => new_id(),
    };
    let name = file
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| source.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "file".into());
    let mime = file.mime.clone().filter(|m| m.contains('/')).unwrap_or_else(|| mime_for(&name).to_string());
    let bytes = std::fs::read(source)?;
    write_local(app, &id, &bytes)?;
    let (width, height) = match (file.width, file.height) {
        (Some(w), Some(h)) => (Some(w), Some(h)),
        _ if mime.starts_with("image/") => image_size(&bytes),
        _ => (None, None),
    };
    Ok(Attachment { id, name, mime, size: metadata.len(), width, height })
}

fn write_local(app: &App, id: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let dir = app.config.files_dir();
    std::fs::create_dir_all(&dir)?;
    crate::config::set_private(&dir)?;
    let path = dir.join(id);
    std::fs::write(&path, bytes)?;
    crate::config::set_private(&path)?;
    Ok(())
}

/// Queues the attachment's bytes as a `file` blob. Call before the chat op that names it, so
/// the relay hands Runners the bytes before the message that needs them.
pub fn push_blob(app: &App, chat_id: Option<&str>, attachment: &Attachment) -> anyhow::Result<()> {
    let Some(dek) = app.dek() else { return Ok(()) };
    let bytes = std::fs::read(local_path(app, &attachment.id))?;
    let ciphertext = crate::crypto::encrypt(&dek, "file", &bytes)?;
    app.push_file_blob(attachment.id.clone(), chat_id, ciphertext);
    Ok(())
}

/// The attachment's bytes on this machine, fetched from the relay when another Device sent it.
pub async fn ensure_local(app: &Arc<App>, attachment: &Attachment) -> anyhow::Result<PathBuf> {
    let path = local_path(app, &attachment.id);
    if path.is_file() {
        return Ok(path);
    }
    let url = app.relay_url().ok_or_else(|| anyhow::anyhow!("no relay configured"))?;
    let machine_file = app.machine_file().ok_or_else(|| anyhow::anyhow!("not paired"))?;
    let machine = machine_file.machine()?;
    let dek = machine_file.dek()?;
    let token = crate::sync::token_or_register(app, &url, &machine).await.map_err(|e| anyhow::anyhow!(e.message))?;
    let ciphertext = app
        .relay
        .get_file(&url, &token, &attachment.id)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?
        .ok_or_else(|| anyhow::anyhow!("the relay no longer has {}", attachment.name))?;
    let bytes = crate::crypto::decrypt(&dek, "file", &ciphertext)?;
    write_local(app, &attachment.id, &bytes)?;
    Ok(path)
}

/// Fetches whatever a turn's transcript refers to that is not here yet. Failures are logged;
/// the turn still runs, and the prompt says the file could not be fetched.
pub async fn prefetch(app: &Arc<App>, attachments: &[Attachment]) {
    for attachment in attachments {
        if is_local(app, &attachment.id) {
            continue;
        }
        if let Err(error) = ensure_local(app, attachment).await {
            tracing::warn!(%error, name = %attachment.name, "fetching an attachment");
        }
    }
}

/// The path a bot reads the attachment at: `<workdir>/attachments/<id>/<name>`, copied from the
/// store on first use. Stable across turns, so the transcript names the same path each time.
pub fn materialize(app: &App, attachment: &Attachment, workdir: &Path) -> Option<PathBuf> {
    let source = local_path(app, &attachment.id);
    if !source.is_file() {
        return None;
    }
    let dir = workdir.join("attachments").join(&attachment.id);
    let target = dir.join(safe_name(&attachment.name));
    if !target.is_file() {
        if let Err(error) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(&source, &target).map(|_| ())) {
            tracing::warn!(%error, name = %attachment.name, "copying an attachment into the workspace");
            return None;
        }
    }
    Some(target)
}

/// What the model sees for one attachment: a line naming the file, and, when `pixels` (the
/// model takes images), an image up to 5 MB as an image part too.
#[cfg(feature = "runner")]
pub fn content_parts(app: &App, attachment: &Attachment, workdir: &Path, pixels: bool) -> Vec<ContentPart> {
    let Some(path) = materialize(app, attachment, workdir) else {
        return vec![ContentPart::text(format!("[Attachment {} ({}) could not be fetched on this Runner]", attachment.name, attachment.mime))];
    };
    let mut parts = vec![ContentPart::text(format!(
        "[Attached: {} ({}, {}) at {}]",
        attachment.name,
        attachment.mime,
        human_size(attachment.size),
        path.display()
    ))];
    if pixels && attachment.is_image() && attachment.size <= MAX_IMAGE_PART_BYTES {
        if let Ok(bytes) = std::fs::read(&path) {
            parts.push(ContentPart::Image {
                data: base64::engine::general_purpose::STANDARD.encode(&bytes),
                mime_type: attachment.mime.clone(),
            });
        }
    }
    parts
}

fn safe_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c == '/' || c == '\\' || c == '\0' { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_start_matches('.');
    if trimmed.is_empty() { "file".into() } else { trimmed.to_string() }
}

pub fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    }
}

pub fn mime_for(name: &str) -> &'static str {
    let extension = name.rsplit('.').next().map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "zip" => "application/zip",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    }
}

/// Pixel size from the header of a PNG, JPEG, GIF, or WebP; the apps size thumbnails from it
/// without decoding the file.
fn image_size(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    fn be32(b: &[u8]) -> u32 {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    }
    if bytes.len() >= 24 && bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return (Some(be32(&bytes[16..20])), Some(be32(&bytes[20..24])));
    }
    if bytes.len() >= 10 && bytes.starts_with(b"GIF8") {
        return (Some(u16::from_le_bytes([bytes[6], bytes[7]]) as u32), Some(u16::from_le_bytes([bytes[8], bytes[9]]) as u32));
    }
    if bytes.len() >= 30 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" && &bytes[12..16] == b"VP8X" {
        let w = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
        let h = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
        return (Some(w), Some(h));
    }
    if bytes.len() > 4 && bytes.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = bytes[i + 1];
            if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                return (Some(w), Some(h));
            }
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            i += 2 + len.max(2);
        }
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_header_size() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(image_size(&png), (Some(640), Some(480)));
        assert_eq!(image_size(b"nope"), (None, None));
    }

    #[test]
    fn mime_and_names() {
        assert_eq!(mime_for("Photo.JPG"), "image/jpeg");
        assert_eq!(mime_for("notes"), "application/octet-stream");
        assert_eq!(safe_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(human_size(2_500_000), "2.4 MB");
    }

    #[test]
    fn text_body_without_attachments_still_parses() {
        let body: crate::model::Body = serde_json::from_str(r#"{"kind":"text","text":"hi"}"#).unwrap();
        assert_eq!(body, crate::model::Body::text("hi"));
        assert_eq!(serde_json::to_string(&body).unwrap(), r#"{"kind":"text","text":"hi"}"#);
    }
}
