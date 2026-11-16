use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use image::{DynamicImage, ImageFormat, RgbImage};
use koushi_state::{
    HistoryExportFailureKind, HistoryExportRange, HistoryExportRoom, HistoryExportRoomCounts,
    HistoryExportRoomFailureKind, HistoryExportRoomPhase,
};
use matrix_sdk::ruma::events::room::MediaSource;
use serde_json::json;

use super::archive::*;
use super::attachments::{AttachmentFetcher, FetchError, StopFlag};
use super::driver::{HistoryPage, HistoryPageError, HistoryPageSource};
use super::element::{ExportHeader, ExportSourceEvent};
use super::fs::{HistoryExportFilesystem, HistoryExportFsError};
use super::fs_fake::MemoryFilesystem;
use super::layout::room_folder_name;
use super::manifest::{ExportManifest, ManifestRoomStatus, ManifestScope};
use super::space_selection::{SelectedRoom, SelectionError};

const CHOSEN: &str = "/out";

fn png() -> Vec<u8> {
    let image = DynamicImage::ImageRgb8(RgbImage::new(8, 8));
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn text(room: &str, index: u64) -> ExportSourceEvent {
    ExportSourceEvent::Plain(json!({
        "type": "m.room.message", "sender": "@alice:x", "room_id": room,
        "event_id": format!("${room}-{index}"), "origin_server_ts": index,
        "content": { "msgtype": "m.text", "body": format!("message {index}") }
    }))
}

fn image(room: &str, index: u64) -> ExportSourceEvent {
    ExportSourceEvent::Plain(json!({
        "type": "m.room.message", "sender": "@alice:x", "room_id": room,
        "event_id": format!("${room}-img{index}"), "origin_server_ts": index,
        "content": { "msgtype": "m.image", "body": format!("cat{index}.png"), "url": format!("mxc://h/{room}{index}") }
    }))
}

struct FakePages(VecDeque<Result<HistoryPage, HistoryPageError>>);

impl HistoryPageSource for FakePages {
    async fn seek(&mut self, _at_ms: u64) -> Result<Option<HistoryPage>, HistoryPageError> {
        Ok(None)
    }

    async fn next_page(&mut self, _from: Option<String>) -> Result<HistoryPage, HistoryPageError> {
        self.0.pop_front().unwrap_or(Ok(HistoryPage {
            events: Vec::new(),
            end: None,
        }))
    }
}

#[derive(Clone, Default)]
struct FakeSource {
    rooms: HashMap<String, Vec<ExportSourceEvent>>,
    failing: Arc<Mutex<BTreeSet<String>>>,
    selection: Arc<Mutex<Vec<SelectedRoom>>>,
    titles: HashMap<String, String>,
    opened: Arc<Mutex<Vec<String>>>,
}

impl FakeSource {
    fn room(mut self, id: &str, events: Vec<ExportSourceEvent>) -> Self {
        self.rooms.insert(id.to_owned(), events);
        self.titles.insert(id.to_owned(), format!("Room {id}"));
        self
    }

    fn space(mut self, id: &str, selection: Vec<SelectedRoom>) -> Self {
        self.titles.insert(id.to_owned(), "Lab".to_owned());
        *self.selection.lock().unwrap() = selection;
        self
    }

    fn opened(&self) -> Vec<String> {
        self.opened.lock().unwrap().clone()
    }
}

impl ArchiveSource for FakeSource {
    type Pages = FakePages;

    fn own_user_id(&self) -> &str {
        "@alice:x"
    }

    async fn scope_title(&mut self, scope: &ManifestScope) -> Option<String> {
        let (ManifestScope::Room { id } | ManifestScope::Space { id }) = scope;
        self.titles.get(id).cloned()
    }

    async fn select_rooms(
        &mut self,
        scope: &ManifestScope,
    ) -> Result<Vec<SelectedRoom>, SelectionError> {
        match scope {
            ManifestScope::Room { id } => Ok(vec![SelectedRoom {
                room_id: id.clone(),
                display_name: format!("Room {id}"),
                target: true,
            }]),
            ManifestScope::Space { .. } => Ok(self.selection.lock().unwrap().clone()),
        }
    }

    async fn open_room(&mut self, room_id: &str) -> Option<(ExportHeader, FakePages)> {
        self.opened.lock().unwrap().push(room_id.to_owned());
        let events = self.rooms.get(room_id)?.clone();
        let header = ExportHeader {
            room_name: format!("Room {room_id}"),
            room_creator: None,
            topic: String::new(),
            export_date: "9/25/2025".to_owned(),
            exported_by: "Alice".to_owned(),
        };
        let page = if self.failing.lock().unwrap().contains(room_id) {
            Err(HistoryPageError::Network)
        } else {
            Ok(HistoryPage { events, end: None })
        };
        Some((header, FakePages(VecDeque::from([page]))))
    }

    async fn sender_names(
        &mut self,
        _room_id: &str,
        senders: &BTreeSet<String>,
    ) -> BTreeMap<String, String> {
        senders
            .iter()
            .map(|sender| (sender.clone(), "Alice".to_owned()))
            .collect()
    }
}

struct PngFetcher;

impl AttachmentFetcher for PngFetcher {
    async fn fetch(&self, _source: &MediaSource) -> Result<Vec<u8>, FetchError> {
        Ok(png())
    }
}

#[derive(Default)]
struct Reporter {
    prepared: Vec<HistoryExportRoom>,
    phases: Vec<(String, HistoryExportRoomPhase)>,
    settled: Vec<(
        String,
        HistoryExportRoomPhase,
        Option<HistoryExportRoomFailureKind>,
    )>,
    /// Set the stop flag when this room reaches this phase.
    stop_at: Option<(String, HistoryExportRoomPhase, StopFlag)>,
    /// Make every later write fail once this room settles.
    fill_disk_after: Option<(String, MemoryFilesystem)>,
}

impl ArchiveReporter for Reporter {
    async fn prepared(&mut self, rooms: Vec<HistoryExportRoom>) {
        self.prepared = rooms;
    }

    fn progressed(
        &mut self,
        room_id: &str,
        phase: HistoryExportRoomPhase,
        _counts: HistoryExportRoomCounts,
    ) {
        self.phases.push((room_id.to_owned(), phase));
        if let Some((room, at, stop)) = &self.stop_at
            && room == room_id
            && *at == phase
        {
            stop.set();
        }
    }

    async fn settled(
        &mut self,
        room_id: &str,
        phase: HistoryExportRoomPhase,
        _counts: HistoryExportRoomCounts,
        failure_kind: Option<HistoryExportRoomFailureKind>,
    ) {
        self.settled.push((room_id.to_owned(), phase, failure_kind));
        if let Some((room, fs)) = &self.fill_disk_after
            && room == room_id
        {
            fs.fail_writes_with(Some(HistoryExportFsError::NoSpace));
        }
    }
}

fn request(scope: ManifestScope, chosen: &str) -> ArchiveRequest {
    ArchiveRequest {
        scope,
        range: HistoryExportRange::AllAvailable,
        chosen_dir: PathBuf::from(chosen),
        folder_name_stem: "Lab".to_owned(),
        folder_date: "2025-09-25".to_owned(),
        time_zone: "UTC".to_owned(),
        now_ms: 1_758_758_400_000,
        labels: super::html::test_labels(),
    }
}

fn space_scope() -> ManifestScope {
    ManifestScope::Space {
        id: "!space".to_owned(),
    }
}

fn selected(id: &str, target: bool) -> SelectedRoom {
    SelectedRoom {
        room_id: id.to_owned(),
        display_name: format!("Room {id}"),
        target,
    }
}

fn filesystem() -> MemoryFilesystem {
    let fs = MemoryFilesystem::default();
    fs.create_dir_all(Path::new(CHOSEN)).unwrap();
    fs
}

struct Outcome {
    result: Result<ArchiveOutcome, HistoryExportFailureKind>,
    dir: Option<PathBuf>,
}

async fn run(
    fs: &MemoryFilesystem,
    source: &mut FakeSource,
    reporter: &mut Reporter,
    stop: &StopFlag,
    request: &ArchiveRequest,
) -> Outcome {
    let mut dir = None;
    let result = run_archive(fs, source, &PngFetcher, reporter, stop, request, &mut dir).await;
    Outcome { result, dir }
}

fn manifest(fs: &MemoryFilesystem, dir: &Path) -> ExportManifest {
    serde_json::from_slice(&fs.read(&dir.join("koushi-export.json")).unwrap()).unwrap()
}

fn statuses(fs: &MemoryFilesystem, dir: &Path) -> Vec<(String, ManifestRoomStatus)> {
    manifest(fs, dir)
        .rooms
        .into_iter()
        .map(|room| (room.room_id, room.status))
        .collect()
}

fn export_dir() -> PathBuf {
    Path::new(CHOSEN).join("Lab - Export 2025-09-25")
}

#[tokio::test]
async fn room_export_writes_full_layout() {
    let fs = filesystem();
    let mut source = FakeSource::default().room("!a", vec![text("!a", 1), image("!a", 2)]);
    let mut reporter = Reporter::default();
    let outcome = run(
        &fs,
        &mut source,
        &mut reporter,
        &StopFlag::default(),
        &request(
            ManifestScope::Room {
                id: "!a".to_owned(),
            },
            CHOSEN,
        ),
    )
    .await;
    assert_eq!(outcome.result, Ok(ArchiveOutcome::Completed));
    let dir = outcome.dir.unwrap();
    assert_eq!(dir, export_dir());
    let folder = room_folder_name("Room !a", "!a");
    let files = fs.files_below(&dir);
    for expected in [
        "koushi-export.json".to_owned(),
        "index.html".to_owned(),
        "assets/koushi-math.js".to_owned(),
        "assets/katex/katex.min.js".to_owned(),
        "assets/katex/fonts/KaTeX_Main-Regular.woff2".to_owned(),
        format!("rooms/{folder}/messages.json"),
        format!("rooms/{folder}/events.jsonl"),
        format!("rooms/{folder}/room.json"),
        format!("rooms/{folder}/attachments.json"),
        format!("rooms/{folder}/index.html"),
        format!("rooms/{folder}/files/0001_cat2.png"),
        format!("rooms/{folder}/thumbs/0001.jpg"),
    ] {
        assert!(
            files.contains(&expected),
            "missing {expected} in {files:#?}"
        );
    }
    assert!(!files.iter().any(|file| file.contains(".partial")));
    assert_eq!(
        statuses(&fs, &dir),
        vec![("!a".to_owned(), ManifestRoomStatus::Completed)]
    );
    let page = String::from_utf8(
        fs.read(&dir.join(format!("rooms/{folder}/index.html")))
            .unwrap(),
    )
    .unwrap();
    assert!(page.contains("message 1"));
    assert!(page.contains("thumbs/0001.jpg"));
    assert_eq!(reporter.prepared.len(), 1);
    assert_eq!(reporter.prepared[0].phase, HistoryExportRoomPhase::Pending);
    let phases: Vec<_> = reporter.phases.iter().map(|(_, phase)| *phase).collect();
    assert!(phases.contains(&HistoryExportRoomPhase::Fetching));
    assert!(phases.contains(&HistoryExportRoomPhase::Attachments));
    assert!(phases.contains(&HistoryExportRoomPhase::Rendering));
    assert_eq!(
        reporter.settled,
        vec![("!a".to_owned(), HistoryExportRoomPhase::Completed, None)]
    );
    let counts = manifest(&fs, &dir).rooms[0].counts;
    assert_eq!(
        (
            counts.exported_events,
            counts.attachments_total,
            counts.attachments_done
        ),
        (2, 1, 1)
    );
}

#[tokio::test]
async fn space_export_skips_unjoined_and_lists_them_in_index() {
    let fs = filesystem();
    let mut source = FakeSource::default()
        .room("!a", vec![text("!a", 1)])
        .space("!space", vec![selected("!a", true), selected("!c", false)]);
    let mut reporter = Reporter::default();
    let outcome = run(
        &fs,
        &mut source,
        &mut reporter,
        &StopFlag::default(),
        &request(space_scope(), CHOSEN),
    )
    .await;
    assert_eq!(outcome.result, Ok(ArchiveOutcome::Completed));
    let dir = outcome.dir.unwrap();
    assert_eq!(
        statuses(&fs, &dir),
        vec![
            ("!a".to_owned(), ManifestRoomStatus::Completed),
            ("!c".to_owned(), ManifestRoomStatus::Skipped)
        ]
    );
    assert_eq!(source.opened(), vec!["!a".to_owned()]);
    assert_eq!(reporter.prepared[1].phase, HistoryExportRoomPhase::Skipped);
    let index = String::from_utf8(fs.read(&dir.join("index.html")).unwrap()).unwrap();
    assert!(index.contains("Room !c"));
    assert!(index.contains("Skipped (not joined)"));
    assert!(
        !fs.files_below(&dir)
            .iter()
            .any(|file| file.contains("Room !c"))
    );
}

#[tokio::test]
async fn stop_then_resume_redoes_only_the_interrupted_room() {
    let fs = filesystem();
    let mut source = FakeSource::default()
        .room("!a", vec![text("!a", 1)])
        .room("!b", vec![image("!b", 1), image("!b", 2)])
        .space("!space", vec![selected("!a", true), selected("!b", true)]);
    let stop = StopFlag::default();
    let mut reporter = Reporter {
        stop_at: Some((
            "!b".to_owned(),
            HistoryExportRoomPhase::Attachments,
            stop.clone(),
        )),
        ..Reporter::default()
    };
    let first = run(
        &fs,
        &mut source,
        &mut reporter,
        &stop,
        &request(space_scope(), CHOSEN),
    )
    .await;
    assert_eq!(first.result, Ok(ArchiveOutcome::Stopped));
    let dir = first.dir.unwrap();
    assert!(
        !fs.files_below(&dir)
            .iter()
            .any(|file| file.contains(".partial")),
        "{:#?}",
        fs.files_below(&dir)
    );
    assert_eq!(
        statuses(&fs, &dir),
        vec![
            ("!a".to_owned(), ManifestRoomStatus::Completed),
            ("!b".to_owned(), ManifestRoomStatus::Pending)
        ]
    );
    assert!(fs.exists(&dir.join("index.html")));

    let mut reporter = Reporter::default();
    let resumed = run(
        &fs,
        &mut source,
        &mut reporter,
        &StopFlag::default(),
        &request(space_scope(), dir.to_str().unwrap()),
    )
    .await;
    assert_eq!(resumed.result, Ok(ArchiveOutcome::Completed));
    assert_eq!(resumed.dir.as_deref(), Some(dir.as_path()));
    assert_eq!(
        source.opened(),
        vec!["!a".to_owned(), "!b".to_owned(), "!b".to_owned()]
    );
    assert_eq!(
        reporter.prepared[0].phase,
        HistoryExportRoomPhase::Completed
    );
    assert_eq!(
        statuses(&fs, &dir),
        vec![
            ("!a".to_owned(), ManifestRoomStatus::Completed),
            ("!b".to_owned(), ManifestRoomStatus::Completed)
        ]
    );
}

#[tokio::test]
async fn room_fetch_failure_continues_with_next_room() {
    let fs = filesystem();
    let mut source = FakeSource::default()
        .room("!a", vec![text("!a", 1)])
        .room("!b", vec![text("!b", 1)])
        .space("!space", vec![selected("!a", true), selected("!b", true)]);
    source.failing.lock().unwrap().insert("!a".to_owned());
    let mut reporter = Reporter::default();
    let outcome = run(
        &fs,
        &mut source,
        &mut reporter,
        &StopFlag::default(),
        &request(space_scope(), CHOSEN),
    )
    .await;
    assert_eq!(outcome.result, Ok(ArchiveOutcome::Completed));
    let dir = outcome.dir.unwrap();
    assert_eq!(
        statuses(&fs, &dir),
        vec![
            ("!a".to_owned(), ManifestRoomStatus::Failed),
            ("!b".to_owned(), ManifestRoomStatus::Completed)
        ]
    );
    assert_eq!(
        reporter.settled[0],
        (
            "!a".to_owned(),
            HistoryExportRoomPhase::Failed,
            Some(HistoryExportRoomFailureKind::Network)
        )
    );
    assert!(
        !fs.files_below(&dir)
            .iter()
            .any(|file| file.contains(".partial"))
    );
}

#[tokio::test]
async fn retry_after_failure_redoes_failed_rooms_only() {
    let fs = filesystem();
    let mut source = FakeSource::default()
        .room("!a", vec![text("!a", 1)])
        .room("!b", vec![text("!b", 1)])
        .space("!space", vec![selected("!a", true), selected("!b", true)]);
    source.failing.lock().unwrap().insert("!a".to_owned());
    let first = run(
        &fs,
        &mut source,
        &mut Reporter::default(),
        &StopFlag::default(),
        &request(space_scope(), CHOSEN),
    )
    .await;
    let dir = first.dir.unwrap();
    source.failing.lock().unwrap().clear();
    source.opened.lock().unwrap().clear();
    let retry = run(
        &fs,
        &mut source,
        &mut Reporter::default(),
        &StopFlag::default(),
        &request(space_scope(), dir.to_str().unwrap()),
    )
    .await;
    assert_eq!(retry.result, Ok(ArchiveOutcome::Completed));
    assert_eq!(source.opened(), vec!["!a".to_owned()]);
    assert_eq!(
        statuses(&fs, &dir),
        vec![
            ("!a".to_owned(), ManifestRoomStatus::Completed),
            ("!b".to_owned(), ManifestRoomStatus::Completed)
        ]
    );
}

#[tokio::test]
async fn no_space_fails_whole_export_and_keeps_completed_rooms() {
    let fs = filesystem();
    let mut source = FakeSource::default()
        .room("!a", vec![text("!a", 1)])
        .room("!b", vec![text("!b", 1)])
        .space("!space", vec![selected("!a", true), selected("!b", true)]);
    let mut reporter = Reporter {
        fill_disk_after: Some(("!a".to_owned(), fs.clone())),
        ..Reporter::default()
    };
    let outcome = run(
        &fs,
        &mut source,
        &mut reporter,
        &StopFlag::default(),
        &request(space_scope(), CHOSEN),
    )
    .await;
    assert_eq!(outcome.result, Err(HistoryExportFailureKind::NoSpace));
    let dir = outcome.dir.unwrap();
    let folder = room_folder_name("Room !a", "!a");
    assert!(fs.exists(&dir.join(format!("rooms/{folder}/index.html"))));
    assert!(
        !fs.files_below(&dir)
            .iter()
            .any(|file| file.contains(".partial"))
    );
    assert_eq!(
        statuses(&fs, &dir)[0],
        ("!a".to_owned(), ManifestRoomStatus::Completed)
    );
}

#[tokio::test]
async fn manifest_mismatch_refuses_resume() {
    let fs = filesystem();
    let other = ExportManifest::new(
        ManifestScope::Space {
            id: "!other".to_owned(),
        },
        HistoryExportRange::AllAvailable,
        "Other".to_owned(),
    );
    fs.write_atomic(
        &Path::new(CHOSEN).join("koushi-export.json"),
        &other.to_bytes(),
    )
    .unwrap();
    let mut source = FakeSource::default().space("!space", vec![]);
    let outcome = run(
        &fs,
        &mut source,
        &mut Reporter::default(),
        &StopFlag::default(),
        &request(space_scope(), CHOSEN),
    )
    .await;
    assert_eq!(
        outcome.result,
        Err(HistoryExportFailureKind::ManifestMismatch)
    );
    assert_eq!(
        manifest(&fs, Path::new(CHOSEN)).title,
        "Other",
        "the other export is untouched"
    );
}

#[tokio::test]
async fn stale_partial_folders_are_removed_on_start() {
    let fs = filesystem();
    let mut source = FakeSource::default()
        .room("!a", vec![text("!a", 1)])
        .space("!space", vec![selected("!a", true)]);
    let first = run(
        &fs,
        &mut source,
        &mut Reporter::default(),
        &StopFlag::default(),
        &request(space_scope(), CHOSEN),
    )
    .await;
    let dir = first.dir.unwrap();
    fs.create_dir_all(&dir.join("rooms/.crashed (00000000).partial"))
        .unwrap();
    fs.write_atomic(
        &dir.join("rooms/.crashed (00000000).partial/messages.json"),
        b"{",
    )
    .unwrap();
    run(
        &fs,
        &mut source,
        &mut Reporter::default(),
        &StopFlag::default(),
        &request(space_scope(), dir.to_str().unwrap()),
    )
    .await;
    assert!(!fs.exists(&dir.join("rooms/.crashed (00000000).partial")));
}

#[tokio::test]
async fn new_space_room_is_added_on_resume() {
    let fs = filesystem();
    let mut source = FakeSource::default()
        .room("!a", vec![text("!a", 1)])
        .room("!b", vec![text("!b", 1)])
        .space("!space", vec![selected("!a", true)]);
    let first = run(
        &fs,
        &mut source,
        &mut Reporter::default(),
        &StopFlag::default(),
        &request(space_scope(), CHOSEN),
    )
    .await;
    let dir = first.dir.unwrap();
    *source.selection.lock().unwrap() = vec![selected("!a", true), selected("!b", true)];
    source.opened.lock().unwrap().clear();
    let second = run(
        &fs,
        &mut source,
        &mut Reporter::default(),
        &StopFlag::default(),
        &request(space_scope(), dir.to_str().unwrap()),
    )
    .await;
    assert_eq!(second.result, Ok(ArchiveOutcome::Completed));
    assert_eq!(source.opened(), vec!["!b".to_owned()]);
    assert_eq!(statuses(&fs, &dir).len(), 2);
}

#[tokio::test]
async fn choosing_the_parent_again_starts_a_new_folder_instead_of_resuming() {
    let fs = filesystem();
    let mut source = FakeSource::default()
        .room("!a", vec![text("!a", 1)])
        .space("!space", vec![selected("!a", true)]);
    run(
        &fs,
        &mut source,
        &mut Reporter::default(),
        &StopFlag::default(),
        &request(space_scope(), CHOSEN),
    )
    .await;
    source.opened.lock().unwrap().clear();
    let again = run(
        &fs,
        &mut source,
        &mut Reporter::default(),
        &StopFlag::default(),
        &request(space_scope(), CHOSEN),
    )
    .await;
    assert_eq!(again.result, Ok(ArchiveOutcome::Completed));
    assert_eq!(
        again.dir,
        Some(Path::new(CHOSEN).join("Lab - Export 2025-09-25 (2)"))
    );
    assert_eq!(
        source.opened(),
        vec!["!a".to_owned()],
        "a new export reads the room again"
    );
}

#[tokio::test]
async fn a_different_export_with_the_same_folder_name_does_not_collide() {
    let fs = filesystem();
    let mut source = FakeSource::default()
        .room("!a", vec![text("!a", 1)])
        .space("!space", vec![selected("!a", true)]);
    run(
        &fs,
        &mut source,
        &mut Reporter::default(),
        &StopFlag::default(),
        &request(space_scope(), CHOSEN),
    )
    .await;
    let mut period = request(space_scope(), CHOSEN);
    period.range = HistoryExportRange::Period {
        start_ms: 0,
        end_exclusive_ms: 10,
        time_zone: "UTC".to_owned(),
    };
    let other = run(
        &fs,
        &mut source,
        &mut Reporter::default(),
        &StopFlag::default(),
        &period,
    )
    .await;
    assert_eq!(other.result, Ok(ArchiveOutcome::Completed));
    assert_eq!(
        other.dir,
        Some(Path::new(CHOSEN).join("Lab - Export 2025-09-25 (2)"))
    );
}

#[tokio::test]
async fn an_unrelated_folder_with_the_same_name_is_not_reused() {
    let fs = filesystem();
    fs.create_dir_all(&export_dir()).unwrap();
    fs.write_atomic(&export_dir().join("notes.txt"), b"mine")
        .unwrap();
    let mut source = FakeSource::default()
        .room("!a", vec![text("!a", 1)])
        .space("!space", vec![selected("!a", true)]);
    let outcome = run(
        &fs,
        &mut source,
        &mut Reporter::default(),
        &StopFlag::default(),
        &request(space_scope(), CHOSEN),
    )
    .await;
    assert_eq!(
        outcome.dir,
        Some(Path::new(CHOSEN).join("Lab - Export 2025-09-25 (2)"))
    );
    assert_eq!(fs.read(&export_dir().join("notes.txt")).unwrap(), b"mine");
}

#[tokio::test]
async fn unknown_room_or_space_fails_before_writing() {
    let fs = filesystem();
    let mut source = FakeSource::default();
    let room = run(
        &fs,
        &mut source,
        &mut Reporter::default(),
        &StopFlag::default(),
        &request(
            ManifestScope::Room {
                id: "!gone".to_owned(),
            },
            CHOSEN,
        ),
    )
    .await;
    assert_eq!(room.result, Err(HistoryExportFailureKind::RoomNotFound));
    let space = run(
        &fs,
        &mut source,
        &mut Reporter::default(),
        &StopFlag::default(),
        &request(space_scope(), CHOSEN),
    )
    .await;
    assert_eq!(space.result, Err(HistoryExportFailureKind::SpaceNotFound));
    assert!(fs.files_below(Path::new(CHOSEN)).is_empty());
}

/// The page Playwright opens over `file://` (e2e/history-export-page.spec.ts).
/// CI's browser job has no Rust toolchain, so the rendered folder is
/// committed; this test fails when the renderer and the fixture drift.
/// Regenerate with `KOUSHI_UPDATE_HISTORY_EXPORT_FIXTURE=1`.
#[tokio::test]
async fn browser_fixture_matches_renderer() {
    // Runtime lookup: `env!` would key the test build on the checkout path.
    let manifest_dir =
        std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo test sets CARGO_MANIFEST_DIR");
    let fixture =
        Path::new(&manifest_dir).join("../../apps/desktop/e2e/fixtures/history-export-page");
    let output = tempfile::tempdir().unwrap();
    let fs = super::fs::NativeHistoryExportFilesystem;
    let math = ExportSourceEvent::Plain(json!({
        "type": "m.room.message", "sender": "@alice:x", "room_id": "!math",
        "event_id": "$math-1", "origin_server_ts": 1_758_758_400_000_u64,
        "content": {
            "msgtype": "m.text",
            "body": "Energy $E=mc^2$ and the integral",
            "format": "org.matrix.custom.html",
            "formatted_body": "Energy <span data-mx-maths=\"E=mc^2\"><code>E=mc^2</code></span> and <div data-mx-maths=\"\\int_0^1 x\\,dx = \\frac{1}{2}\"><code>\\int_0^1 x\\,dx</code></div>"
        }
    }));
    let picture = ExportSourceEvent::Plain(json!({
        "type": "m.room.message", "sender": "@alice:x", "room_id": "!math",
        "event_id": "$math-2", "origin_server_ts": 1_758_758_460_000_u64,
        "content": { "msgtype": "m.image", "body": "plot.png", "url": "mxc://h/plot" }
    }));
    let mut source = FakeSource::default().room("!math", vec![math, picture]);
    let mut request = request(
        ManifestScope::Room {
            id: "!math".to_owned(),
        },
        output.path().to_str().unwrap(),
    );
    request.folder_name_stem = "Fixture".to_owned();
    request.time_zone = "Asia/Tokyo".to_owned();
    let mut dir = None;
    let result = run_archive(
        &fs,
        &mut source,
        &PngFetcher,
        &mut Reporter::default(),
        &StopFlag::default(),
        &request,
        &mut dir,
    )
    .await;
    assert_eq!(result, Ok(ArchiveOutcome::Completed));
    let dir = dir.unwrap();

    let rendered = relative_files(&dir);
    if std::env::var_os("KOUSHI_UPDATE_HISTORY_EXPORT_FIXTURE").is_some() {
        let _ = std::fs::remove_dir_all(&fixture);
        for file in &rendered {
            let target = fixture.join(file);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::copy(dir.join(file), target).unwrap();
        }
    }
    assert_eq!(
        rendered,
        relative_files(&fixture),
        "fixture file list drifted; regenerate it"
    );
    for file in &rendered {
        assert_eq!(
            std::fs::read(dir.join(file)).unwrap(),
            std::fs::read(fixture.join(file)).unwrap(),
            "{file} drifted; regenerate the fixture"
        );
    }
}

/// Files below `root` except the vendored `assets/`, sorted, `/`-separated.
fn relative_files(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut files = Vec::new();
    walk(root, root, &mut files);
    files.retain(|file| !file.starts_with("assets/"));
    files.sort();
    files
}

#[tokio::test]
async fn a_room_the_filesystem_rejects_fails_alone_and_the_export_continues() {
    let fs = filesystem();
    let mut source = FakeSource::default()
        .room("!a", vec![text("!a", 1)])
        .room("!b", vec![text("!b", 1)])
        .space("!space", vec![selected("!a", true), selected("!b", true)]);
    fs.fail_paths_containing(&room_folder_name("Room !a", "!a"), HistoryExportFsError::Io);
    let mut reporter = Reporter::default();
    let outcome = run(
        &fs,
        &mut source,
        &mut reporter,
        &StopFlag::default(),
        &request(space_scope(), CHOSEN),
    )
    .await;
    assert_eq!(outcome.result, Ok(ArchiveOutcome::Completed));
    let dir = outcome.dir.unwrap();
    assert_eq!(
        statuses(&fs, &dir),
        vec![
            ("!a".to_owned(), ManifestRoomStatus::Failed),
            ("!b".to_owned(), ManifestRoomStatus::Completed)
        ]
    );
    assert_eq!(
        reporter.settled[0],
        (
            "!a".to_owned(),
            HistoryExportRoomPhase::Failed,
            Some(HistoryExportRoomFailureKind::Write)
        )
    );
}

#[tokio::test]
async fn a_permission_error_still_fails_the_whole_export() {
    let fs = filesystem();
    let mut source = FakeSource::default()
        .room("!a", vec![text("!a", 1)])
        .space("!space", vec![selected("!a", true)]);
    fs.fail_paths_containing(
        &room_folder_name("Room !a", "!a"),
        HistoryExportFsError::PermissionDenied,
    );
    let outcome = run(
        &fs,
        &mut source,
        &mut Reporter::default(),
        &StopFlag::default(),
        &request(space_scope(), CHOSEN),
    )
    .await;
    assert_eq!(outcome.result, Err(HistoryExportFailureKind::Write));
}
