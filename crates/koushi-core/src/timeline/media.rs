use std::collections::HashMap;
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
}

pub(super) struct MediaDownloadReady {
    download_state: TimelineMediaDownloadState,
    source_url: String,
    byte_count: u64,
    mimetype: Option<String>,
    width: Option<u64>,
    height: Option<u64>,
}

pub(super) enum MediaDownloadOutcome {
    Ready(MediaDownloadReady),
    Failed(TimelineFailureKind),
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

        let Some(entry) = self.media_sources.get(&event_id).cloned() else {
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
        let dir_name = sanitize_matrix_id_for_path(&room_id);
        let file_name = format!("{}.bin", sanitize_matrix_id_for_path(&event_id));
        let dir = data_dir.join("media-downloads").join(dir_name);
        let path = dir.join(file_name);
        if let Ok(metadata) = tokio::fs::metadata(&path).await
            && metadata.is_file()
            && metadata.len() > 0
        {
            let byte_count = metadata.len();
            let source_url = path.to_string_lossy().into_owned();
            trace_media_download_worker("cache_hit", &request, Some(byte_count), None, None);
            return MediaDownloadOutcome::Ready(MediaDownloadReady {
                download_state: TimelineMediaDownloadState::Ready {
                    source_url: source_url.clone(),
                    width: entry.width,
                    height: entry.height,
                    mime_type: entry.mimetype.clone(),
                },
                source_url,
                byte_count,
                mimetype: entry.mimetype,
                width: entry.width,
                height: entry.height,
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

        let source_url = path.to_string_lossy().into_owned();
        trace_media_download_worker("completed", &request, Some(byte_count), None, None);
        MediaDownloadOutcome::Ready(MediaDownloadReady {
            download_state: TimelineMediaDownloadState::Ready {
                source_url: source_url.clone(),
                width: entry.width,
                height: entry.height,
                mime_type: entry.mimetype.clone(),
            },
            source_url,
            byte_count,
            mimetype: entry.mimetype,
            width: entry.width,
            height: entry.height,
        })
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
                    source_url: ready.source_url,
                    byte_count: ready.byte_count,
                    mimetype: ready.mimetype,
                    width: ready.width,
                    height: ready.height,
                }));
                self.emit_action_reliable(AppAction::MediaDownloadUpdated {
                    room_id: self.key.room_id().to_owned(),
                    event_id,
                    state: ready.download_state,
                })
                .await;
            }
            MediaDownloadOutcome::Failed(kind) => {
                self.emit_download_failed(request_id, &event_id, kind).await;
            }
        }
    }
    async fn emit_media_download_current_state(&self, request_id: RequestId, event_id: &str) {
        let total = self
            .media_sources
            .get(event_id)
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
            | VectorDiff::PopBack => {}
        }
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
mod tests {

    use koushi_state::{AppAction, TimelineMediaKind as GalleryTimelineMediaKind};

    use matrix_sdk::attachment::AttachmentInfo;

    use koushi_protocol::command::{UploadMediaKind, UploadMediaRequest};
    use koushi_protocol::event::{TimelineDiff, TimelineItemId, TimelineMediaKind};

    use matrix_sdk::ruma::uint;

    use koushi_protocol::command::{
        ImageUploadCompressionPolicy, ImageUploadCompressionState, ImageUploadDimensions,
        ImageUploadVariantInfo, ImageUploadVariantKind,
    };

    use super::super::display_projection::apply_timeline_diffs_to_items;
    use super::super::item_projection::attachment_info_for_upload;
    use super::super::test_support::{room_key, timeline_item, timeline_media_item};
    use super::{authoritative_media_gallery_replacement, media_gallery_items_from_timeline_items};

    #[test]
    fn media_gallery_projection_keeps_event_media_newest_first() {
        let mut transaction_media = timeline_media_item(
            "$local:test",
            "@me:test",
            None,
            3,
            "local.png",
            TimelineMediaKind::Image,
        );
        transaction_media.id = TimelineItemId::Transaction {
            transaction_id: "txn-local".to_owned(),
        };
        let items = vec![
            timeline_media_item(
                "$old:test",
                "@alice:test",
                Some("Alice"),
                1,
                "old.png",
                TimelineMediaKind::Image,
            ),
            timeline_item("$text:test", Some("text"), "@bob:test", false),
            transaction_media,
            timeline_media_item(
                "$new:test",
                "@carol:test",
                Some("Carol"),
                2,
                "new.png",
                TimelineMediaKind::Image,
            ),
        ];

        let gallery = media_gallery_items_from_timeline_items(&room_key(), &items);

        assert_eq!(gallery.len(), 2);
        assert_eq!(gallery[0].event_id, "$new:test");
        assert_eq!(gallery[0].sender.as_deref(), Some("@carol:test"));
        assert_eq!(gallery[0].sender_label.as_deref(), Some("Carol"));
        assert_eq!(gallery[0].timestamp_ms, 2);
        assert_eq!(gallery[0].media.kind, GalleryTimelineMediaKind::Image);
        assert_eq!(gallery[0].media.filename, "new.png");
        assert!(gallery[0].media.source.encrypted);
        assert_eq!(
            gallery[0].media.thumbnail.as_ref().map(|thumb| thumb.width),
            Some(Some(160))
        );
        assert_eq!(gallery[1].event_id, "$old:test");
    }

    #[test]
    fn media_gallery_projection_recomputes_after_timeline_diffs() {
        let mut items = vec![
            timeline_media_item(
                "$old:test",
                "@alice:test",
                None,
                1,
                "old.png",
                TimelineMediaKind::Image,
            ),
            timeline_media_item(
                "$new:test",
                "@bob:test",
                None,
                2,
                "new.png",
                TimelineMediaKind::Image,
            ),
        ];

        apply_timeline_diffs_to_items(&mut items, &[TimelineDiff::Remove { index: 1 }]);
        let gallery = media_gallery_items_from_timeline_items(&room_key(), &items);
        assert_eq!(gallery.len(), 1);
        assert_eq!(gallery[0].event_id, "$old:test");

        apply_timeline_diffs_to_items(&mut items, &[TimelineDiff::Reset { items: Vec::new() }]);
        assert!(media_gallery_items_from_timeline_items(&room_key(), &items).is_empty());
    }

    #[test]
    fn relay_overflow_authoritative_snapshot_replaces_media_gallery_and_emits_action() {
        let key = room_key();
        let old_navigation = vec![timeline_media_item(
            "$old:test",
            "@alice:test",
            None,
            1,
            "old.png",
            TimelineMediaKind::Image,
        )];
        let new_navigation = vec![timeline_media_item(
            "$new:test",
            "@bob:test",
            None,
            2,
            "new.png",
            TimelineMediaKind::Image,
        )];
        let old_gallery = media_gallery_items_from_timeline_items(&key, &old_navigation);
        let replacement =
            authoritative_media_gallery_replacement(&key, &old_gallery, &new_navigation)
                .expect("changed authoritative snapshot must emit gallery replacement");
        assert_eq!(replacement.items.len(), 1);
        assert_eq!(replacement.items[0].event_id, "$new:test");
        assert!(matches!(
            replacement.action,
            AppAction::MediaGalleryUpdated { items, .. }
                if items.len() == 1 && items[0].event_id == "$new:test"
        ));
    }

    #[test]
    fn attachment_info_for_image_upload_uses_selected_variant_metadata() {
        let request = UploadMediaRequest {
            filename: "private-screenshot.jpg".to_owned(),
            mime_type: "image/jpeg".to_owned(),
            bytes: vec![1, 2, 3, 4],
            kind: UploadMediaKind::Image {
                width: Some(1200),
                height: Some(900),
            },
            compression: Some(ImageUploadCompressionState {
                mode: koushi_state::ImageUploadCompressionMode::Always,
                policy: ImageUploadCompressionPolicy::default(),
                original: ImageUploadVariantInfo {
                    mime_type: "image/jpeg".to_owned(),
                    byte_count: 3_200_000,
                    dimensions: Some(ImageUploadDimensions {
                        width: 4032,
                        height: 3024,
                    }),
                },
                selected: ImageUploadVariantInfo {
                    mime_type: "image/jpeg".to_owned(),
                    byte_count: 4,
                    dimensions: Some(ImageUploadDimensions {
                        width: 1200,
                        height: 900,
                    }),
                },
                selected_variant: ImageUploadVariantKind::Compressed,
                skipped_small_image: false,
                metadata_stripped: true,
                thumbnail_refreshed: true,
            }),
            thumbnail: None,
            caption: None,
        };

        match attachment_info_for_upload(&request) {
            AttachmentInfo::Image(info) => {
                assert_eq!(info.width, Some(uint!(1200)));
                assert_eq!(info.height, Some(uint!(900)));
                assert_eq!(info.size, Some(uint!(4)));
            }
            other => panic!("expected image info, got {other:?}"),
        }
    }
}
