use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
use koushi_sdk::MatrixClientSession;
use koushi_state::{
    AppAction, MediaTransferProgress, OperationFailureKind, TimelineMediaDownloadState,
    TimelineMediaGalleryItem, TimelineMediaGalleryMedia, TimelineMediaGallerySource,
    TimelineMediaGalleryThumbnail, TimelineMediaKind as GalleryTimelineMediaKind,
};

use matrix_sdk::media::{MediaFormat, MediaRequestParameters};
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk_ui::timeline::TimelineItem as SdkTimelineItem;

use crate::executor;
use crate::media_download_ref::{
    media_download_cache_path, media_download_ref, media_download_thumbnail_ref,
};
use koushi_protocol::command::MediaDownloadSelection;
use koushi_protocol::event::{
    CoreEvent, TimelineEvent, TimelineItem, TimelineItemId, TimelineMediaKind, TimelineMediaSource,
    TimelineMediaThumbnail,
};
use koushi_protocol::failure::TimelineFailureKind;
use koushi_protocol::ids::{RequestId, TimelineKey, TimelineKind};

// BEGIN GENERATED SIBLING IMPORTS
use super::actor::{TimelineActor, TimelineActorMessage};
use super::item_projection::{
    cache_sdk_item_media_source, media_request_for_download, sanitize_matrix_id_for_path,
};
// END GENERATED SIBLING IMPORTS

const MEDIA_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub(super) struct PrivateMediaEntry {
    pub(super) source: MediaSource,
    pub(super) thumbnail_source: Option<MediaSource>,
    pub(super) mimetype: Option<String>,
    pub(super) size: u64,
    pub(super) width: Option<u64>,
    pub(super) height: Option<u64>,
    pub(super) thumbnail_mimetype: Option<String>,
    pub(super) thumbnail_width: Option<u64>,
    pub(super) thumbnail_height: Option<u64>,
}

impl std::fmt::Debug for PrivateMediaEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PrivateMediaEntry")
            .field("source", &"MediaSource(..)")
            .field(
                "thumbnail_source",
                &self.thumbnail_source.as_ref().map(|_| "MediaSource(..)"),
            )
            .field("mimetype", &self.mimetype)
            .field("size", &self.size)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("thumbnail_mimetype", &self.thumbnail_mimetype)
            .field("thumbnail_width", &self.thumbnail_width)
            .field("thumbnail_height", &self.thumbnail_height)
            .finish()
    }
}

pub(super) struct MediaDownloadReady {
    download_state: TimelineMediaDownloadState,
    /// Opaque cache handle for Core state/events; the local path stays private.
    source_ref: String,
    /// Managed cache file, kept actor-private for save/open callers.
    path: PathBuf,
    byte_count: u64,
    mimetype: Option<String>,
    width: Option<u64>,
    height: Option<u64>,
}

pub(super) enum MediaDownloadOutcome {
    Ready(MediaDownloadReady),
    Failed(TimelineFailureKind),
}

/// The image sniffers need at most 12 bytes; 32 leaves room for a future kind.
const MEDIA_DOWNLOAD_MIME_SNIFF_BYTES: usize = 32;

/// #1874: the timeline renders an image reply quote as a preview of at most
/// 240x160 CSS px, so the quote thumbnail is fetched at 2x that box for HiDPI
/// screens (server `Scale` semantics keep the aspect ratio inside it).
pub(super) const REPLY_QUOTE_PREVIEW_SELECTION: MediaDownloadSelection =
    MediaDownloadSelection::Thumbnail {
        width: 480,
        height: 360,
    };

/// A cached download has no event metadata to copy a mime type from, so read
/// just the header. Renderable images must carry their type for the platform
/// protocol that serves them (#1619).
async fn sniff_image_mime(path: &Path) -> Option<String> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await.ok()?;
    let mut header = [0u8; MEDIA_DOWNLOAD_MIME_SNIFF_BYTES];
    let read = file.read(&mut header).await.ok()?;
    koushi_media::image_kind(&header[..read]).map(|kind| kind.mime_type.to_owned())
}

fn media_download_selection_token(selection: &MediaDownloadSelection) -> &'static str {
    match selection {
        MediaDownloadSelection::File => "file",
        MediaDownloadSelection::Thumbnail { .. } => "thumbnail",
    }
}

fn media_download_failure_token(kind: TimelineFailureKind) -> &'static str {
    match kind {
        TimelineFailureKind::InvalidDirection => "invalid_direction",
        TimelineFailureKind::InvalidReactionTarget => "invalid_reaction_target",
        TimelineFailureKind::InvalidReactionState => "invalid_reaction_state",
        TimelineFailureKind::InvalidSendTarget => "invalid_send_target",
        TimelineFailureKind::InvalidSendState => "invalid_send_state",
        TimelineFailureKind::SecureBackupRequired => "secure_backup_required",
        TimelineFailureKind::ComposerRevisionExhausted => "composer_revision_exhausted",
        TimelineFailureKind::UnsupportedSlashCommand => "unsupported_slash_command",
        TimelineFailureKind::NotSubscribed => "not_subscribed",
        TimelineFailureKind::Forbidden => "forbidden",
        TimelineFailureKind::NotFound => "not_found",
        TimelineFailureKind::Network => "network",
        TimelineFailureKind::Timeout => "timeout",
        TimelineFailureKind::Sdk => "sdk",
        TimelineFailureKind::QueueOverflow => "queue_overflow",
    }
}

fn media_source_token(source: &MediaSource) -> &'static str {
    match source {
        MediaSource::Plain(_) => "plain",
        MediaSource::Encrypted(_) => "encrypted",
    }
}

fn media_format_token(format: &MediaFormat) -> &'static str {
    match format {
        MediaFormat::File => "file",
        MediaFormat::Thumbnail(_) => "thumbnail",
    }
}

fn sdk_media_error_token(error: &matrix_sdk::Error) -> &'static str {
    match error {
        matrix_sdk::Error::Http(_) => "http",
        matrix_sdk::Error::AuthenticationRequired => "authentication_required",
        matrix_sdk::Error::InsufficientData => "insufficient_data",
        matrix_sdk::Error::SerdeJson(_) => "serde_json",
        matrix_sdk::Error::Io(_) => "io",
        matrix_sdk::Error::CrossProcessLockError(_) => "cross_process_lock",
        matrix_sdk::Error::StateStore(_) => "state_store",
        matrix_sdk::Error::EventCacheStore(_) => "event_cache_store",
        matrix_sdk::Error::MediaStore(_) => "media_store",
        matrix_sdk::Error::Identifier(_) => "identifier",
        matrix_sdk::Error::Url(_) => "url",
        matrix_sdk::Error::UserTagName(_) => "user_tag_name",
        matrix_sdk::Error::SlidingSync(_) => "sliding_sync",
        matrix_sdk::Error::WrongRoomState(_) => "wrong_room_state",
        matrix_sdk::Error::MultipleSessionCallbacks => "multiple_session_callbacks",
        matrix_sdk::Error::OAuth(_) => "oauth",
        matrix_sdk::Error::ConcurrentRequestFailed => "concurrent_request_failed",
        matrix_sdk::Error::UnknownError(_) => "unknown",
        matrix_sdk::Error::EventCache(_) => "event_cache",
        matrix_sdk::Error::SendQueueWedgeError(_) => "send_queue_wedge",
        matrix_sdk::Error::BackupNotEnabled => "backup_not_enabled",
        matrix_sdk::Error::CantIgnoreLoggedInUser => "cant_ignore_logged_in_user",
        matrix_sdk::Error::Media(_) => "media",
        matrix_sdk::Error::ReplyError(_) => "reply",
        matrix_sdk::Error::PowerLevels(_) => "power_levels",
        matrix_sdk::Error::Timeout => "timeout",
        _ => "other",
    }
}

fn trace_media_download_request(
    stage: &'static str,
    request_id: RequestId,
    selection: &MediaDownloadSelection,
    entry: Option<&PrivateMediaEntry>,
    outcome: Option<&'static str>,
) {
    let mut event = DiagnosticEvent::new(DiagnosticLevel::Info, "core.media_download", stage)
        .field(DiagnosticField::request_id(
            "request_id",
            request_id.connection_id.0,
            request_id.sequence,
        ))
        .field(DiagnosticField::token(
            "selection",
            media_download_selection_token(selection),
        ));
    if let Some(entry) = entry {
        event = event
            .field(DiagnosticField::token(
                "source",
                media_source_token(&entry.source),
            ))
            .field(DiagnosticField::boolean(
                "source_encrypted",
                matches!(entry.source, MediaSource::Encrypted(_)),
            ))
            .field(DiagnosticField::boolean(
                "thumbnail_source_present",
                entry.thumbnail_source.is_some(),
            ))
            .field(DiagnosticField::count("declared_size", entry.size));
    }
    if let Some(outcome) = outcome {
        event = event.field(DiagnosticField::token("outcome", outcome));
    }
    koushi_diagnostics::record(event);
}

fn trace_media_download_worker(
    stage: &'static str,
    request: &MediaRequestParameters,
    byte_count: Option<u64>,
    failure: Option<&'static str>,
    sdk_error: Option<&'static str>,
) {
    let mut event = DiagnosticEvent::new(DiagnosticLevel::Info, "core.media_download", stage)
        .field(DiagnosticField::token(
            "source",
            media_source_token(&request.source),
        ))
        .field(DiagnosticField::boolean(
            "source_encrypted",
            matches!(request.source, MediaSource::Encrypted(_)),
        ))
        .field(DiagnosticField::token(
            "format",
            media_format_token(&request.format),
        ));
    if let Some(byte_count) = byte_count {
        event = event.field(DiagnosticField::count("byte_count", byte_count));
    }
    if let Some(failure) = failure {
        event = event.field(DiagnosticField::token("failure", failure));
    }
    if let Some(sdk_error) = sdk_error {
        event = event.field(DiagnosticField::token("sdk_error", sdk_error));
    }
    koushi_diagnostics::record(event);
}

fn trace_media_download_file_write_failed(
    request: &MediaRequestParameters,
    byte_count: u64,
    failure: &'static str,
    error: Option<&std::io::Error>,
    data_dir_present: bool,
    target_dir: Option<&Path>,
    target_path: Option<&Path>,
) {
    let mut event = DiagnosticEvent::new(
        DiagnosticLevel::Info,
        "core.media_download",
        "file_write_failed",
    )
    .field(DiagnosticField::token(
        "source",
        media_source_token(&request.source),
    ))
    .field(DiagnosticField::boolean(
        "source_encrypted",
        matches!(request.source, MediaSource::Encrypted(_)),
    ))
    .field(DiagnosticField::token(
        "format",
        media_format_token(&request.format),
    ))
    .field(DiagnosticField::count("byte_count", byte_count))
    .field(DiagnosticField::token("failure", failure))
    .field(DiagnosticField::boolean(
        "data_dir_present",
        data_dir_present,
    ))
    .field(DiagnosticField::boolean(
        "target_dir_exists",
        target_dir.is_some_and(Path::exists),
    ))
    .field(DiagnosticField::boolean(
        "target_path_exists",
        target_path.is_some_and(Path::exists),
    ))
    .field(DiagnosticField::boolean(
        "target_path_is_file",
        target_path.is_some_and(Path::is_file),
    ))
    .field(DiagnosticField::boolean(
        "target_path_is_dir",
        target_path.is_some_and(Path::is_dir),
    ));
    if let Some(error) = error {
        event = event.field(DiagnosticField::token(
            "io_error_kind",
            io_error_kind_token(error.kind()),
        ));
        if let Some(raw_os_error) = error.raw_os_error()
            && let Ok(raw_os_error) = u64::try_from(raw_os_error)
        {
            event = event.field(DiagnosticField::count("raw_os_error", raw_os_error));
        }
    }
    koushi_diagnostics::record(event);
}

fn io_error_kind_token(kind: std::io::ErrorKind) -> &'static str {
    match kind {
        std::io::ErrorKind::NotFound => "not_found",
        std::io::ErrorKind::PermissionDenied => "permission_denied",
        std::io::ErrorKind::AlreadyExists => "already_exists",
        std::io::ErrorKind::InvalidInput => "invalid_input",
        std::io::ErrorKind::InvalidData => "invalid_data",
        std::io::ErrorKind::TimedOut => "timed_out",
        std::io::ErrorKind::WriteZero => "write_zero",
        std::io::ErrorKind::Interrupted => "interrupted",
        std::io::ErrorKind::UnexpectedEof => "unexpected_eof",
        std::io::ErrorKind::OutOfMemory => "out_of_memory",
        std::io::ErrorKind::Other => "other",
        _ => "unknown",
    }
}

fn reply_quote_thumbnail_fetch_is_pending(
    tasks: &HashMap<String, executor::JoinHandle<()>>,
    quoted_event_id: &str,
) -> bool {
    tasks.contains_key(quoted_event_id)
}

/// Resolve a quoted original's private source: resident rows use
/// `media_sources`, an off-window original uses the hydration ledger.
pub(super) fn reply_quote_media_source(
    media_sources: &HashMap<String, PrivateMediaEntry>,
    hydration: &super::reply_quote_hydration::ReplyQuoteHydration,
    quoted_event_id: &str,
) -> Option<PrivateMediaEntry> {
    media_sources
        .get(quoted_event_id)
        .cloned()
        .or_else(|| hydration.settled_media_source(quoted_event_id).cloned())
}

impl PrivateMediaEntry {
    /// Whether two entries name the same bytes, for stale-worker guards.
    pub(super) fn has_same_source(&self, other: &Self) -> bool {
        fn identity(source: &MediaSource) -> (&str, bool) {
            match source {
                MediaSource::Plain(uri) => (uri.as_str(), false),
                MediaSource::Encrypted(file) => (file.url.as_str(), true),
            }
        }
        identity(&self.source) == identity(&other.source)
    }
}

/// The safe metadata a selection's bytes carry. A thumbnail with an explicit
/// source uses the thumbnail's own type and dimensions; a server-generated
/// thumbnail has no reliable dimensions, so they stay unknown (#1769).
fn media_download_selection_metadata(
    entry: &PrivateMediaEntry,
    selection: &MediaDownloadSelection,
) -> (Option<String>, Option<u64>, Option<u64>) {
    match selection {
        MediaDownloadSelection::File => (entry.mimetype.clone(), entry.width, entry.height),
        MediaDownloadSelection::Thumbnail { .. } => (
            entry.thumbnail_mimetype.clone(),
            entry.thumbnail_width,
            entry.thumbnail_height,
        ),
    }
}

/// Remove every cache entry a reply-quote thumbnail can be answered from, so
/// an edit that swaps the quoted media cannot leave the old picture behind.
async fn remove_reply_quote_cached_media(data_dir: &Path, room_id: &str, quoted_event_id: &str) {
    let room_hash = sanitize_matrix_id_for_path(room_id);
    let event_hash = sanitize_matrix_id_for_path(quoted_event_id);
    for selection in [MediaDownloadSelection::File, REPLY_QUOTE_PREVIEW_SELECTION] {
        if let Some((_, path)) =
            managed_media_download_entry(data_dir, &room_hash, &event_hash, &selection)
        {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}

/// A full file already cached for this event is a usable quote thumbnail.
async fn reply_quote_thumbnail_from_cached_file(
    entry: &PrivateMediaEntry,
    data_dir: &Path,
    room_id: &str,
    quoted_event_id: &str,
) -> Option<MediaDownloadReady> {
    let room_hash = sanitize_matrix_id_for_path(room_id);
    let event_hash = sanitize_matrix_id_for_path(quoted_event_id);
    let (source_ref, path) = managed_media_download_entry(
        data_dir,
        &room_hash,
        &event_hash,
        &MediaDownloadSelection::File,
    )?;
    let metadata = tokio::fs::metadata(&path).await.ok()?;
    if !metadata.is_file() || metadata.len() == 0 {
        return None;
    }
    let byte_count = metadata.len();
    let mimetype = match entry.mimetype.clone() {
        Some(mimetype) => Some(mimetype),
        None => sniff_image_mime(&path).await,
    };
    // A cached video or audio original is not a renderable quote thumbnail;
    // only a still image may stand in for the preview box. An animated GIF or
    // WebP must go through the thumbnail selection so the quote stays static.
    if !mimetype.as_deref().is_some_and(|mimetype| {
        mimetype.starts_with("image/") && !matches!(mimetype, "image/gif" | "image/webp")
    }) {
        return None;
    }
    Some(MediaDownloadReady {
        download_state: TimelineMediaDownloadState::Ready {
            source_ref: source_ref.clone(),
            width: entry.width,
            height: entry.height,
            mime_type: mimetype.clone(),
        },
        source_ref,
        path,
        byte_count,
        mimetype,
        width: entry.width,
        height: entry.height,
    })
}

impl TimelineActor {
    pub(super) fn sdk_room_for_key(&self) -> Option<matrix_sdk::Room> {
        let room_id_str = match &self.key.kind {
            TimelineKind::Room { room_id }
            | TimelineKind::Thread { room_id, .. }
            | TimelineKind::Focused { room_id, .. } => room_id,
        };
        let room_id = matrix_sdk::ruma::RoomId::parse(room_id_str).ok()?;
        self.session.client().get_room(&room_id)
    }
    pub(super) async fn handle_download_media(
        &mut self,
        request_id: RequestId,
        event_id: String,
        selection: MediaDownloadSelection,
    ) {
        trace_media_download_request(
            "request_received",
            request_id,
            &selection,
            self.media_sources.get(&event_id),
            None,
        );
        if self.media_downloads_in_progress.contains(&event_id) {
            trace_media_download_request(
                "request_rejected",
                request_id,
                &selection,
                self.media_sources.get(&event_id),
                Some("already_in_progress"),
            );
            self.emit_media_download_current_state(request_id, &event_id)
                .await;
            return;
        }

        // An off-window quoted original is not in `media_sources`; the
        // hydration ledger keeps its private source so the viewer can open it
        // (#1769).
        let Some(entry) = self.reply_quote_media_source(&event_id) else {
            trace_media_download_request(
                "request_rejected",
                request_id,
                &selection,
                None,
                Some("missing_media_source"),
            );
            self.emit_download_failed(request_id, &event_id, TimelineFailureKind::Sdk)
                .await;
            return;
        };

        let Some(request) = media_request_for_download(&entry, &selection) else {
            trace_media_download_request(
                "request_rejected",
                request_id,
                &selection,
                Some(&entry),
                Some("unsupported_request"),
            );
            self.emit_download_failed(request_id, &event_id, TimelineFailureKind::Sdk)
                .await;
            return;
        };

        self.media_downloads_in_progress.insert(event_id.clone());
        self.emit_media_download_current_state(request_id, &event_id)
            .await;

        let session = self.session.clone();
        let room_id = self.key.room_id().to_owned();
        let data_dir = self.data_dir.clone();
        let actor_tx = self.msg_tx.clone();
        let event_id_for_task = event_id.clone();
        let task = executor::spawn(async move {
            let outcome = Self::download_media_for(
                session,
                data_dir,
                room_id,
                event_id_for_task.clone(),
                entry,
                &selection,
                request,
            )
            .await;
            let _ = actor_tx
                .send(TimelineActorMessage::MediaDownloadFinished {
                    request_id,
                    event_id: event_id_for_task,
                    outcome,
                })
                .await;
        });
        self.media_download_tasks.insert(event_id, task);
    }
    async fn download_media_for(
        session: Arc<MatrixClientSession>,
        data_dir: Option<PathBuf>,
        room_id: String,
        event_id: String,
        entry: PrivateMediaEntry,
        selection: &MediaDownloadSelection,
        request: MediaRequestParameters,
    ) -> MediaDownloadOutcome {
        let Some(data_dir) = data_dir else {
            trace_media_download_file_write_failed(
                &request,
                0,
                "missing_data_dir",
                None,
                false,
                None,
                None,
            );
            return MediaDownloadOutcome::Failed(TimelineFailureKind::Sdk);
        };

        // Matrix IDs contain ':' which is not valid in Windows path components.
        // Use hashed path components so the local path is portable and private.
        let room_hash = sanitize_matrix_id_for_path(&room_id);
        let event_hash = sanitize_matrix_id_for_path(&event_id);
        // The ref names the managed entry but carries no path: only the hashed
        // components are visible to React and the platform adapter (#1619).
        let Some((source_ref, path)) =
            managed_media_download_entry(&data_dir, &room_hash, &event_hash, selection)
        else {
            return MediaDownloadOutcome::Failed(TimelineFailureKind::Sdk);
        };
        let dir = data_dir.join("media-downloads").join(&room_hash);
        // A hit means this selection was requested before, so a thumbnail no
        // longer answers a `File` request (#1770). The `File` name is unchanged,
        // so existing caches keep hitting — including one a pre-fix thumbnail
        // already overwrote, which keeps serving that JPEG until it is cleared.
        if let Ok(metadata) = tokio::fs::metadata(&path).await
            && metadata.is_file()
            && metadata.len() > 0
        {
            let byte_count = metadata.len();
            let (declared_mimetype, width, height) =
                media_download_selection_metadata(&entry, selection);
            let mimetype = match declared_mimetype {
                Some(mimetype) => Some(mimetype),
                None => sniff_image_mime(&path).await,
            };
            trace_media_download_worker("cache_hit", &request, Some(byte_count), None, None);
            return MediaDownloadOutcome::Ready(MediaDownloadReady {
                download_state: TimelineMediaDownloadState::Ready {
                    source_ref: source_ref.clone(),
                    width,
                    height,
                    mime_type: mimetype.clone(),
                },
                source_ref,
                path,
                byte_count,
                mimetype,
                width,
                height,
            });
        }

        trace_media_download_worker("sdk_fetch_started", &request, None, None, None);
        let bytes = match executor::timeout(
            MEDIA_DOWNLOAD_TIMEOUT,
            session.client().media().get_media_content(&request, true),
        )
        .await
        {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(error)) => {
                let kind = classify_media_download_error(&error);
                trace_media_download_worker(
                    "sdk_fetch_failed",
                    &request,
                    None,
                    Some(media_download_failure_token(kind)),
                    Some(sdk_media_error_token(&error)),
                );
                return MediaDownloadOutcome::Failed(kind);
            }
            Err(_) => {
                trace_media_download_worker(
                    "sdk_fetch_failed",
                    &request,
                    None,
                    Some(media_download_failure_token(TimelineFailureKind::Timeout)),
                    Some("timeout"),
                );
                return MediaDownloadOutcome::Failed(TimelineFailureKind::Timeout);
            }
        };

        let byte_count = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if let Err(error) = tokio::fs::create_dir_all(&dir).await {
            trace_media_download_file_write_failed(
                &request,
                byte_count,
                "create_dir",
                Some(&error),
                true,
                Some(&dir),
                None,
            );
            return MediaDownloadOutcome::Failed(TimelineFailureKind::Sdk);
        }
        if let Err(error) = tokio::fs::write(&path, &bytes).await {
            trace_media_download_file_write_failed(
                &request,
                byte_count,
                "write_file",
                Some(&error),
                true,
                Some(&dir),
                Some(&path),
            );
            return MediaDownloadOutcome::Failed(TimelineFailureKind::Sdk);
        }

        let (declared_mimetype, width, height) =
            media_download_selection_metadata(&entry, selection);
        let mimetype = match declared_mimetype {
            Some(mimetype) => Some(mimetype),
            None => koushi_media::image_kind(&bytes).map(|kind| kind.mime_type.to_owned()),
        };
        trace_media_download_worker("completed", &request, Some(byte_count), None, None);
        MediaDownloadOutcome::Ready(MediaDownloadReady {
            download_state: TimelineMediaDownloadState::Ready {
                source_ref: source_ref.clone(),
                width,
                height,
                mime_type: mimetype.clone(),
            },
            source_ref,
            path,
            byte_count,
            mimetype,
            width,
            height,
        })
    }
    /// #1769: the private source for a quoted original. A resident original
    /// uses `media_sources`; an off-window one uses the hydration ledger.
    fn reply_quote_media_source(&self, quoted_event_id: &str) -> Option<PrivateMediaEntry> {
        reply_quote_media_source(
            &self.media_sources,
            &self.reply_quote_hydration,
            quoted_event_id,
        )
    }

    pub(super) async fn handle_request_reply_quote_thumbnail(
        &mut self,
        request_id: RequestId,
        quoted_event_id: String,
    ) {
        if reply_quote_thumbnail_fetch_is_pending(
            &self.reply_quote_thumbnail_tasks,
            &quoted_event_id,
        ) {
            self.emit_reply_quote_thumbnail_current_state(request_id, &quoted_event_id)
                .await;
            return;
        }

        let Some(entry) = self.reply_quote_media_source(&quoted_event_id) else {
            self.emit_reply_quote_thumbnail_failed(
                request_id,
                &quoted_event_id,
                TimelineFailureKind::Sdk,
            )
            .await;
            return;
        };

        // A full file already cached for this event is a usable preview and
        // must not cost another fetch.
        if let Some(data_dir) = self.data_dir.clone()
            && let Some(ready) = reply_quote_thumbnail_from_cached_file(
                &entry,
                &data_dir,
                self.key.room_id(),
                &quoted_event_id,
            )
            .await
        {
            self.emit_reply_quote_thumbnail_ready(request_id, &quoted_event_id, ready)
                .await;
            return;
        }

        let selection = REPLY_QUOTE_PREVIEW_SELECTION;
        let Some(request) = media_request_for_download(&entry, &selection) else {
            self.emit_reply_quote_thumbnail_failed(
                request_id,
                &quoted_event_id,
                TimelineFailureKind::Sdk,
            )
            .await;
            return;
        };

        self.emit_reply_quote_thumbnail_current_state(request_id, &quoted_event_id)
            .await;

        let session = self.session.clone();
        let room_id = self.key.room_id().to_owned();
        let data_dir = self.data_dir.clone();
        let actor_tx = self.msg_tx.clone();
        let quoted_event_id_for_task = quoted_event_id.clone();
        let source = entry.clone();
        let task = executor::spawn(async move {
            let outcome = Self::download_media_for(
                session,
                data_dir,
                room_id,
                quoted_event_id_for_task.clone(),
                entry,
                &selection,
                request,
            )
            .await;
            let _ = actor_tx
                .send(TimelineActorMessage::ReplyQuoteThumbnailFinished {
                    request_id,
                    quoted_event_id: quoted_event_id_for_task,
                    source,
                    outcome,
                })
                .await;
        });
        self.reply_quote_thumbnail_tasks
            .insert(quoted_event_id, task);
    }

    /// #1769: an edit that replaces the quoted media must not leave an old
    /// cached file or published thumbnail answering the new source.
    pub(super) async fn invalidate_reply_quote_thumbnail(&mut self, quoted_event_id: &str) {
        if let Some(task) = self.reply_quote_thumbnail_tasks.remove(quoted_event_id) {
            task.abort();
        }
        self.emit_reply_quote_thumbnail_cleared(quoted_event_id)
            .await;
        if let Some(data_dir) = self.data_dir.clone() {
            remove_reply_quote_cached_media(&data_dir, self.key.room_id(), quoted_event_id).await;
        }
    }

    async fn emit_reply_quote_thumbnail_current_state(
        &self,
        request_id: RequestId,
        quoted_event_id: &str,
    ) {
        let total = self
            .reply_quote_media_source(quoted_event_id)
            .map(|entry| entry.size)
            .unwrap_or(0);
        let progress = MediaTransferProgress { current: 0, total };
        self.emit(CoreEvent::Timeline(TimelineEvent::MediaDownloadProgress {
            request_id,
            key: self.key.clone(),
            event_id: quoted_event_id.to_owned(),
            progress,
        }));
        self.emit_action_reliable(AppAction::ReplyQuoteThumbnailUpdated {
            room_id: self.key.room_id().to_owned(),
            quoted_event_id: quoted_event_id.to_owned(),
            state: TimelineMediaDownloadState::Pending {
                progress: Some(progress),
            },
        })
        .await;
    }

    pub(super) async fn handle_reply_quote_thumbnail_finished(
        &mut self,
        request_id: RequestId,
        quoted_event_id: String,
        source: PrivateMediaEntry,
        outcome: MediaDownloadOutcome,
    ) {
        self.reply_quote_thumbnail_tasks.remove(&quoted_event_id);
        let still_current = self
            .reply_quote_media_source(&quoted_event_id)
            .is_some_and(|current| current.has_same_source(&source));
        if !still_current {
            self.emit_reply_quote_thumbnail_cleared(&quoted_event_id)
                .await;
            return;
        }
        match outcome {
            MediaDownloadOutcome::Ready(ready) => {
                self.emit_reply_quote_thumbnail_ready(request_id, &quoted_event_id, ready)
                    .await;
            }
            MediaDownloadOutcome::Failed(kind) => {
                self.emit_reply_quote_thumbnail_failed(request_id, &quoted_event_id, kind)
                    .await;
            }
        }
    }

    async fn emit_reply_quote_thumbnail_cleared(&self, quoted_event_id: &str) {
        self.emit_action_reliable(AppAction::ReplyQuoteThumbnailUpdated {
            room_id: self.key.room_id().to_owned(),
            quoted_event_id: quoted_event_id.to_owned(),
            state: TimelineMediaDownloadState::NotRequested,
        })
        .await;
    }

    async fn emit_reply_quote_thumbnail_ready(
        &self,
        request_id: RequestId,
        quoted_event_id: &str,
        ready: MediaDownloadReady,
    ) {
        self.emit(CoreEvent::Timeline(TimelineEvent::MediaDownloadCompleted {
            request_id,
            key: self.key.clone(),
            event_id: quoted_event_id.to_owned(),
            source_ref: ready.source_ref,
            byte_count: ready.byte_count,
            mimetype: ready.mimetype,
            width: ready.width,
            height: ready.height,
        }));
        self.emit_action_reliable(AppAction::ReplyQuoteThumbnailUpdated {
            room_id: self.key.room_id().to_owned(),
            quoted_event_id: quoted_event_id.to_owned(),
            state: ready.download_state,
        })
        .await;
    }

    async fn emit_reply_quote_thumbnail_failed(
        &self,
        request_id: RequestId,
        quoted_event_id: &str,
        kind: TimelineFailureKind,
    ) {
        self.emit(CoreEvent::Timeline(TimelineEvent::MediaDownloadFailed {
            request_id,
            key: self.key.clone(),
            event_id: quoted_event_id.to_owned(),
            kind,
        }));
        self.emit_action_reliable(AppAction::ReplyQuoteThumbnailUpdated {
            room_id: self.key.room_id().to_owned(),
            quoted_event_id: quoted_event_id.to_owned(),
            state: TimelineMediaDownloadState::Failed {
                failure_kind: match kind {
                    TimelineFailureKind::Network => OperationFailureKind::Network,
                    TimelineFailureKind::Sdk => OperationFailureKind::Sdk,
                    TimelineFailureKind::Forbidden => OperationFailureKind::Forbidden,
                    TimelineFailureKind::Timeout => OperationFailureKind::Timeout,
                    _ => OperationFailureKind::Sdk,
                },
            },
        })
        .await;
    }

    pub(super) async fn handle_media_download_finished(
        &mut self,
        request_id: RequestId,
        event_id: String,
        outcome: MediaDownloadOutcome,
    ) {
        self.media_downloads_in_progress.remove(&event_id);
        self.media_download_tasks.remove(&event_id);
        match outcome {
            MediaDownloadOutcome::Ready(ready) => {
                self.emit(CoreEvent::Timeline(TimelineEvent::MediaDownloadCompleted {
                    request_id,
                    key: self.key.clone(),
                    event_id: event_id.clone(),
                    source_ref: ready.source_ref,
                    byte_count: ready.byte_count,
                    mimetype: ready.mimetype,
                    width: ready.width,
                    height: ready.height,
                }));
                self.emit_action_reliable(AppAction::MediaDownloadUpdated {
                    room_id: self.key.room_id().to_owned(),
                    event_id: event_id.clone(),
                    state: ready.download_state,
                })
                .await;
            }
            MediaDownloadOutcome::Failed(kind) => {
                self.emit_download_failed(request_id, &event_id, kind).await;
            }
        }
        // A download can outlive its timeline item: the removal-batch
        // reconciliation keeps in-flight entries, so completion releases the
        // metadata once nothing needs it anymore.
        release_removed_media_source(&mut self.media_sources, &self.navigation_items, &event_id);
    }
    async fn emit_media_download_current_state(&self, request_id: RequestId, event_id: &str) {
        let total = self
            .reply_quote_media_source(event_id)
            .map(|entry| entry.size)
            .unwrap_or(0);
        let progress = MediaTransferProgress { current: 0, total };
        self.emit(CoreEvent::Timeline(TimelineEvent::MediaDownloadProgress {
            request_id,
            key: self.key.clone(),
            event_id: event_id.to_owned(),
            progress,
        }));
        self.emit_action_reliable(AppAction::MediaDownloadUpdated {
            room_id: self.key.room_id().to_owned(),
            event_id: event_id.to_owned(),
            state: TimelineMediaDownloadState::Pending {
                progress: Some(progress),
            },
        })
        .await;
    }
    async fn emit_download_failed(
        &self,
        request_id: RequestId,
        event_id: &str,
        kind: TimelineFailureKind,
    ) {
        trace_media_download_request(
            "failed_projected",
            request_id,
            &MediaDownloadSelection::File,
            self.media_sources.get(event_id),
            Some(media_download_failure_token(kind)),
        );
        self.emit(CoreEvent::Timeline(TimelineEvent::MediaDownloadFailed {
            request_id,
            key: self.key.clone(),
            event_id: event_id.to_owned(),
            kind,
        }));
        // Use reliable delivery — a dropped failure action leaves the UI stuck
        // in a pending download state (REPOSITORY_RULES State-Machine Discipline).
        self.emit_action_reliable(AppAction::MediaDownloadUpdated {
            room_id: self.key.room_id().to_owned(),
            event_id: event_id.to_owned(),
            state: TimelineMediaDownloadState::Failed {
                failure_kind: match kind {
                    TimelineFailureKind::Network => OperationFailureKind::Network,
                    TimelineFailureKind::Sdk => OperationFailureKind::Sdk,
                    TimelineFailureKind::Forbidden => OperationFailureKind::Forbidden,
                    TimelineFailureKind::Timeout => OperationFailureKind::Timeout,
                    _ => OperationFailureKind::Sdk,
                },
            },
        })
        .await;
    }
    pub(super) async fn emit_media_gallery_if_changed(&mut self) {
        let items = media_gallery_items_from_timeline_items(&self.key, &self.navigation_items);
        if items == self.media_gallery_items {
            return;
        }
        if let Some(action) = media_gallery_updated_action(&self.key, items)
            && !self.emit_action_reliable(action).await
        {
            return;
        }
        self.media_gallery_items =
            media_gallery_items_from_timeline_items(&self.key, &self.navigation_items);
    }
    pub(super) fn apply_sdk_media_cache_diff(
        media_sources: &mut HashMap<String, PrivateMediaEntry>,
        diff: &eyeball_im::VectorDiff<Arc<SdkTimelineItem>>,
    ) {
        use eyeball_im::VectorDiff;

        match diff {
            VectorDiff::PushFront { value }
            | VectorDiff::PushBack { value }
            | VectorDiff::Insert { value, .. }
            | VectorDiff::Set { value, .. } => {
                cache_sdk_item_media_source(media_sources, value);
            }
            VectorDiff::Append { values } => {
                for item in values {
                    cache_sdk_item_media_source(media_sources, item);
                }
            }
            VectorDiff::Reset { values } => {
                media_sources.clear();
                for item in values {
                    cache_sdk_item_media_source(media_sources, item);
                }
            }
            VectorDiff::Clear => {
                media_sources.clear();
            }
            VectorDiff::Remove { .. }
            | VectorDiff::Truncate { .. }
            | VectorDiff::PopFront
            | VectorDiff::PopBack => {
                // The diff does not carry the removed values, so the caller
                // reconciles the map against the resident window after the
                // batch; see `retain_resident_media_sources`.
            }
        }
    }
}

/// Collect the quote media a batch names in one traversal: each event's own
/// private source and public descriptor, plus any media embedded in a reply to
/// it (#1769). `Some(None)` records that the event was observed without media.
pub(super) fn apply_sdk_reply_quote_media_diff(
    sources: &mut HashMap<String, Option<PrivateMediaEntry>>,
    descriptors: &mut HashMap<String, Option<koushi_state::ReplyQuoteMedia>>,
    diff: &eyeball_im::VectorDiff<Arc<SdkTimelineItem>>,
) {
    use eyeball_im::VectorDiff;

    fn observe(
        sources: &mut HashMap<String, Option<PrivateMediaEntry>>,
        descriptors: &mut HashMap<String, Option<koushi_state::ReplyQuoteMedia>>,
        event_id: String,
        content: &matrix_sdk_ui::timeline::TimelineItemContent,
    ) {
        sources.insert(
            event_id.clone(),
            super::item_projection::private_media_entry_from_content(content),
        );
        descriptors.insert(
            event_id,
            super::item_projection::reply_quote_media_from_content(content),
        );
    }

    fn observe_item(
        sources: &mut HashMap<String, Option<PrivateMediaEntry>>,
        descriptors: &mut HashMap<String, Option<koushi_state::ReplyQuoteMedia>>,
        item: &Arc<SdkTimelineItem>,
    ) {
        let Some(event_item) = item.as_event() else {
            return;
        };
        let content = event_item.content();
        if let Some(event_id) = event_item.event_id() {
            observe(sources, descriptors, event_id.to_string(), content);
        }
        let Some(details) = content.in_reply_to() else {
            return;
        };
        let matrix_sdk_ui::timeline::TimelineDetails::Ready(embedded) = &details.event else {
            return;
        };
        observe(
            sources,
            descriptors,
            details.event_id.to_string(),
            &embedded.content,
        );
    }

    match diff {
        VectorDiff::PushFront { value }
        | VectorDiff::PushBack { value }
        | VectorDiff::Insert { value, .. }
        | VectorDiff::Set { value, .. } => observe_item(sources, descriptors, value),
        VectorDiff::Append { values } => {
            for item in values {
                observe_item(sources, descriptors, item);
            }
        }
        VectorDiff::Reset { values } => {
            sources.clear();
            descriptors.clear();
            for item in values {
                observe_item(sources, descriptors, item);
            }
        }
        VectorDiff::Clear => {
            sources.clear();
            descriptors.clear();
        }
        VectorDiff::Remove { .. }
        | VectorDiff::Truncate { .. }
        | VectorDiff::PopFront
        | VectorDiff::PopBack => {}
    }
}

/// Whether this diff can leave a media entry without a resident item.
pub(super) fn sdk_media_diff_removes_items(
    diff: &eyeball_im::VectorDiff<Arc<SdkTimelineItem>>,
) -> bool {
    use eyeball_im::VectorDiff;

    matches!(
        diff,
        VectorDiff::Remove { .. }
            | VectorDiff::Truncate { .. }
            | VectorDiff::PopFront
            | VectorDiff::PopBack
    )
}

/// Drop media metadata for events that are no longer resident.
///
/// `media_sources` mirrors the resident projection, and SDK removal diffs do
/// not name the removed events, so the caller passes the committed window.
/// An entry whose download is still in flight is kept: the worker and the
/// progress events still read it after the item has left the window.
pub(super) fn retain_resident_media_sources(
    media_sources: &mut HashMap<String, PrivateMediaEntry>,
    resident_event_ids: &HashSet<String>,
    downloads_in_progress: &HashSet<String>,
) {
    media_sources.retain(|event_id, _| {
        resident_event_ids.contains(event_id) || downloads_in_progress.contains(event_id)
    });
}

/// Release `event_id`'s media metadata when its event is no longer resident.
///
/// Completion-time counterpart of `retain_resident_media_sources`: a download
/// that outlived its timeline item was kept while in flight, so the finished
/// worker must release it once the item is gone.
pub(super) fn release_removed_media_source(
    media_sources: &mut HashMap<String, PrivateMediaEntry>,
    resident_items: &[TimelineItem],
    event_id: &str,
) {
    let resident = resident_items.iter().any(|item| {
        matches!(&item.id, TimelineItemId::Event { event_id: resident } if resident == event_id)
    });
    if !resident {
        media_sources.remove(event_id);
    }
}

/// The managed cache entry for one download selection: its opaque ref plus the
/// file the platform adapter resolves that same ref to (#1770).
///
/// `None` when the requested size cannot be named in the ref grammar; only a
/// caller bug reaches it, and it fails closed like an unsupported request.
fn managed_media_download_entry(
    data_dir: &Path,
    room_hash: &str,
    event_hash: &str,
    selection: &MediaDownloadSelection,
) -> Option<(String, PathBuf)> {
    let source_ref = match selection {
        MediaDownloadSelection::File => media_download_ref(room_hash, event_hash),
        MediaDownloadSelection::Thumbnail { width, height } => {
            media_download_thumbnail_ref(room_hash, event_hash, *width, *height)?
        }
    };
    // One rule names the file: the resolver the adapter uses, so a written entry
    // is always one an adapter can find again.
    let path = media_download_cache_path(&data_dir.join("media-downloads"), &source_ref)?;
    Some((source_ref, path))
}

/// Download one media event into the account's managed cache, without a
/// resident timeline (#1524). Returns the cache path of the written file.
pub(super) async fn download_media_to_cache(
    session: Arc<MatrixClientSession>,
    data_dir: std::path::PathBuf,
    room_id: String,
    event_id: String,
    entry: PrivateMediaEntry,
    selection: &MediaDownloadSelection,
) -> Result<(std::path::PathBuf, u64, Option<String>), TimelineFailureKind> {
    let Some(request) = super::item_projection::media_request_for_download(&entry, selection)
    else {
        return Err(TimelineFailureKind::Sdk);
    };
    let outcome = TimelineActor::download_media_for(
        session,
        Some(data_dir),
        room_id,
        event_id,
        entry,
        selection,
        request,
    )
    .await;
    match outcome {
        MediaDownloadOutcome::Ready(ready) => Ok((ready.path, ready.byte_count, ready.mimetype)),
        MediaDownloadOutcome::Failed(kind) => Err(kind),
    }
}

pub(super) fn media_gallery_updated_action(
    key: &TimelineKey,
    items: Vec<TimelineMediaGalleryItem>,
) -> Option<AppAction> {
    let TimelineKind::Room { room_id } = &key.kind else {
        return None;
    };

    Some(AppAction::MediaGalleryUpdated {
        room_id: room_id.clone(),
        items,
    })
}

pub(super) struct AuthoritativeMediaGalleryReplacement {
    pub(super) items: Vec<TimelineMediaGalleryItem>,
    pub(super) action: AppAction,
}

pub(super) fn authoritative_media_gallery_replacement(
    key: &TimelineKey,
    current: &[TimelineMediaGalleryItem],
    authoritative_items: &[TimelineItem],
) -> Option<AuthoritativeMediaGalleryReplacement> {
    let items = media_gallery_items_from_timeline_items(key, authoritative_items);
    if items == current {
        return None;
    }
    let action = media_gallery_updated_action(key, items.clone())?;
    Some(AuthoritativeMediaGalleryReplacement { items, action })
}

pub(super) fn media_gallery_items_from_timeline_items(
    key: &TimelineKey,
    items: &[TimelineItem],
) -> Vec<TimelineMediaGalleryItem> {
    let TimelineKind::Room { room_id } = &key.kind else {
        return Vec::new();
    };

    let mut gallery_items = items
        .iter()
        .filter_map(|item| media_gallery_item_from_timeline_item(room_id, item))
        .collect::<Vec<_>>();
    gallery_items.sort_by(|left, right| {
        right
            .timestamp_ms
            .cmp(&left.timestamp_ms)
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    gallery_items
}

fn media_gallery_item_from_timeline_item(
    room_id: &str,
    item: &TimelineItem,
) -> Option<TimelineMediaGalleryItem> {
    if item.is_hidden || item.is_redacted {
        return None;
    }
    let TimelineItemId::Event { event_id } = &item.id else {
        return None;
    };
    let media = item.media.as_ref()?;

    Some(TimelineMediaGalleryItem {
        event_id: event_id.clone(),
        room_id: room_id.to_owned(),
        sender: item.sender.clone(),
        sender_label: item.sender_label.clone(),
        timestamp_ms: item.timestamp_ms.unwrap_or_default(),
        media: TimelineMediaGalleryMedia {
            kind: media_gallery_kind_from_timeline_kind(media.kind),
            filename: media.filename.clone(),
            source: media_gallery_source_from_timeline_source(&media.source),
            mimetype: media.mimetype.clone(),
            size: media.size,
            width: media.width,
            height: media.height,
            thumbnail: media.thumbnail.as_ref().map(media_gallery_thumbnail),
        },
    })
}

fn media_gallery_kind_from_timeline_kind(kind: TimelineMediaKind) -> GalleryTimelineMediaKind {
    match kind {
        TimelineMediaKind::Image => GalleryTimelineMediaKind::Image,
        TimelineMediaKind::File => GalleryTimelineMediaKind::File,
        TimelineMediaKind::Audio => GalleryTimelineMediaKind::Audio,
        TimelineMediaKind::Video => GalleryTimelineMediaKind::Video,
    }
}

fn media_gallery_source_from_timeline_source(
    source: &TimelineMediaSource,
) -> TimelineMediaGallerySource {
    TimelineMediaGallerySource {
        mxc_uri: source.mxc_uri.clone(),
        encrypted: source.encrypted,
        encryption_version: source.encryption_version.clone(),
    }
}

fn media_gallery_thumbnail(thumbnail: &TimelineMediaThumbnail) -> TimelineMediaGalleryThumbnail {
    TimelineMediaGalleryThumbnail {
        source: media_gallery_source_from_timeline_source(&thumbnail.source),
        mimetype: thumbnail.mimetype.clone(),
        size: thumbnail.size,
        width: thumbnail.width,
        height: thumbnail.height,
    }
}

fn classify_media_download_error(error: &matrix_sdk::Error) -> TimelineFailureKind {
    if matches!(error, matrix_sdk::Error::Timeout) {
        return TimelineFailureKind::Timeout;
    }
    if error
        .client_api_error_kind()
        .map(|kind| kind == &matrix_sdk::ruma::api::error::ErrorKind::Forbidden)
        .unwrap_or(false)
    {
        return TimelineFailureKind::Forbidden;
    }
    if matches!(error, matrix_sdk::Error::Http(_)) {
        return TimelineFailureKind::Network;
    }
    TimelineFailureKind::Sdk
}

#[cfg(test)]
mod tests;
