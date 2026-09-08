use koushi_protocol::view::{ReaderRow, ReaderWindowLimit, ResolvedReaderAnchor, ViewModel};

use super::ScopeError;
use crate::view_budget::{ViewBudget, ViewReservation};

const MAX_MODEL_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn encoded_bytes(value: &impl serde::Serialize) -> Result<usize, ScopeError> {
    let mut counter = EncodedSize { bytes: 0 };
    serde_json::to_writer(&mut counter, value).map_err(|_| ScopeError::Capacity)?;
    Ok(counter.bytes)
}

pub(crate) struct PreparedModel {
    pub(crate) model: ViewModel,
    #[cfg(test)]
    encoded_bytes: usize,
    _bytes: ViewReservation,
    pub(super) installed: std::sync::Arc<InstalledRows>,
}

pub(super) struct InstalledRows {
    pub(super) rows: Vec<InstalledRow>,
    pub(super) resources: Vec<crate::timeline::ReaderAvatarResource>,
    _bytes: ViewReservation,
}

pub(super) struct InstalledRow {
    pub(super) user_id: String,
}

fn visit_rows(model: &ViewModel, mut visit: impl FnMut(&ReaderRow)) {
    match model {
        ViewModel::ReaderLoading { .. } => {}
        ViewModel::ReaderReady(window) => {
            for row in &window.rows {
                visit(row);
            }
        }
        ViewModel::TimelineReceipts { summaries, .. } => {
            for summary in summaries {
                for row in &summary.readers {
                    visit(row);
                }
            }
        }
    }
}

fn source_ref(row: &ReaderRow) -> Option<&str> {
    match row.avatar.as_ref() {
        Some(koushi_state::AvatarThumbnailState::Ready { source_ref, .. }) => Some(source_ref),
        _ => None,
    }
}

/// Charge retained encoded data without allocating a second serialized copy.
/// The caller must separately reserve raw/builder data before constructing the model.
pub(super) fn prepare(
    model: ViewModel,
    budget: &ViewBudget,
    mut resources: Vec<crate::timeline::ReaderAvatarResource>,
) -> Result<PreparedModel, ScopeError> {
    if !valid_shape(&model) {
        return Err(ScopeError::InvalidModel);
    }
    let counter = EncodedSize {
        bytes: encoded_bytes(&model)?,
    };
    let reservation = budget
        .reserve_bytes(counter.bytes)
        .ok_or(ScopeError::Capacity)?;
    let mut count = 0;
    let mut metadata_bytes = Some(std::mem::size_of::<InstalledRows>());
    visit_rows(&model, |row| {
        count += 1;
        for bytes in [std::mem::size_of::<InstalledRow>(), row.user_id.len()] {
            metadata_bytes = metadata_bytes.and_then(|total| total.checked_add(bytes));
        }
    });
    if resources.len() > count {
        return Err(ScopeError::InvalidModel);
    }
    resources.sort_unstable_by(|a, b| a.user_id.cmp(&b.user_id));
    if resources
        .windows(2)
        .any(|pair| pair[0].user_id == pair[1].user_id)
    {
        return Err(ScopeError::InvalidModel);
    }
    if resources.iter().any(|resource| {
        matches!(
            &resource.lease,
            Err(crate::renderable_thumbnail::ThumbnailLeaseError::Capacity)
        )
    }) {
        return Err(ScopeError::Capacity);
    }
    for resource in &resources {
        for bytes in [
            std::mem::size_of_val(resource),
            resource.user_id.len(),
            resource.mxc_uri.len(),
            resource
                .lease
                .as_ref()
                .map_or(0, |lease| lease.control_bytes()),
        ] {
            metadata_bytes = metadata_bytes.and_then(|total| total.checked_add(bytes));
        }
    }
    let metadata_reservation = budget
        .reserve_bytes(metadata_bytes.ok_or(ScopeError::Capacity)?)
        .ok_or(ScopeError::Capacity)?;
    let mut valid_resources = true;
    let mut referenced = vec![false; resources.len()];
    visit_rows(&model, |row| {
        if row.avatar.is_none() {
            return;
        }
        let Ok(index) = resources.binary_search_by(|resource| resource.user_id.cmp(&row.user_id))
        else {
            valid_resources = false;
            return;
        };
        referenced[index] = true;
        if let Some(source) = source_ref(row) {
            valid_resources &= resources[index]
                .lease
                .as_ref()
                .is_ok_and(|lease| lease.source_ref() == source);
        } else {
            valid_resources &= resources[index].lease.is_err();
        }
    });
    if !valid_resources || referenced.iter().any(|seen| !seen) {
        return Err(ScopeError::InvalidModel);
    }
    let mut rows = Vec::with_capacity(count);
    visit_rows(&model, |row| {
        rows.push(InstalledRow {
            user_id: row.user_id.clone(),
        })
    });
    Ok(PreparedModel {
        model,
        #[cfg(test)]
        encoded_bytes: counter.bytes,
        _bytes: reservation,
        installed: std::sync::Arc::new(InstalledRows {
            rows,
            resources,
            _bytes: metadata_reservation,
        }),
    })
}

struct EncodedSize {
    bytes: usize,
}

impl std::io::Write for EncodedSize {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let total = self
            .bytes
            .checked_add(bytes.len())
            .filter(|total| *total <= MAX_MODEL_BYTES)
            .ok_or_else(|| std::io::Error::other("scoped model exceeds encoded capacity"))?;
        self.bytes = total;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Structural admission only: source liveness, byte budgets and visibility are separate gates.
pub(super) fn valid_shape(model: &ViewModel) -> bool {
    match model {
        ViewModel::ReaderLoading { .. } => true,
        ViewModel::TimelineReceipts { source, summaries } => {
            bounded_count(summaries.len())
                && summaries.iter().enumerate().all(|(index, summary)| {
                    let count = if summary.total_count <= 4 {
                        summary.total_count
                    } else {
                        3
                    };
                    summary.source.timeline == *source
                        && summary.readers.len() as u64 == count
                        && summary.overflow_count == summary.total_count - count
                        && unique_rows(&summary.readers)
                        && !summaries[..index]
                            .iter()
                            .any(|previous| previous.source.event_id == summary.source.event_id)
                })
        }
        ViewModel::ReaderReady(window) => {
            if !bounded_count(window.rows.len())
                || window.start > window.total_count
                || window.rows.len() as u64 > window.total_count - window.start
                || !unique_rows(&window.rows)
            {
                return false;
            }
            match &window.resolved_anchor {
                ResolvedReaderAnchor::Row { user_id, index } => index
                    .checked_sub(window.start)
                    .and_then(|offset| usize::try_from(offset).ok())
                    .and_then(|offset| window.rows.get(offset))
                    .is_some_and(|row| row.user_id == *user_id),
                ResolvedReaderAnchor::NoSurvivingInstalledRow
                | ResolvedReaderAnchor::NotRequested => true,
            }
        }
    }
}

fn bounded_count(count: usize) -> bool {
    count == 0
        || u16::try_from(count)
            .ok()
            .and_then(|count| ReaderWindowLimit::try_from(count).ok())
            .is_some()
}

fn unique_rows(rows: &[ReaderRow]) -> bool {
    rows.iter().enumerate().all(|(index, row)| {
        !rows[..index]
            .iter()
            .any(|previous| previous.user_id == row.user_id)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use koushi_protocol::view::{ReaderRow, ReceiptCompactSummary, ReceiptSourceRef};

    #[test]
    fn prepared_model_retains_its_exact_encoded_charge() {
        let source = serde_json::from_value(serde_json::json!({
            "key": {"account_key": "account", "kind": {"Room": {"room_id": "!r:example.org"}}},
            "projection_request_id": {"connection_id": "1", "sequence": "2"},
            "generation": "3", "event_id": "$event"
        }))
        .unwrap();
        let model = ViewModel::ReaderLoading { source };
        let expected = serde_json::to_vec(&model).unwrap().len();
        let budget = crate::view_budget::ViewBudget::default();
        let prepared = prepare(model, &budget, Vec::new()).unwrap();
        assert_eq!(prepared.encoded_bytes, expected);
        assert!(matches!(prepared.model, ViewModel::ReaderLoading { .. }));
        let remaining = budget
            .reserve_bytes(256 * 1024 * 1024 - expected - std::mem::size_of::<InstalledRows>())
            .unwrap();
        assert!(budget.reserve_bytes(1).is_none());
        drop(prepared);
        assert!(budget.reserve_bytes(expected).is_some());
        drop(remaining);
        let mut counter = EncodedSize {
            bytes: MAX_MODEL_BYTES - 1,
        };
        assert_eq!(std::io::Write::write(&mut counter, b"x").unwrap(), 1);
        assert!(std::io::Write::write(&mut counter, b"x").is_err());
        assert_eq!(counter.bytes, MAX_MODEL_BYTES);
    }

    #[test]
    fn resource_lease_capacity_retires_before_reader_ready_publication() {
        let source = serde_json::from_value(serde_json::json!({
            "key": {"account_key": "account", "kind": {"Room": {"room_id": "!r:example.org"}}},
            "projection_request_id": {"connection_id": "1", "sequence": "2"},
            "generation": "3", "event_id": "$event"
        }))
        .unwrap();
        let mxc_uri = "mxc://example.org/avatar".to_owned();
        let model = ViewModel::ReaderReady(koushi_protocol::view::ReaderWindow {
            source,
            total_count: 1,
            start: 0,
            rows: vec![ReaderRow {
                user_id: "@reader:example.org".to_owned(),
                display_label: "Reader".to_owned(),
                original_display_label: "Reader".to_owned(),
                initials: "R".to_owned(),
                timestamp: None,
                avatar: Some(koushi_state::AvatarThumbnailState::NotRequested),
            }],
            window_sequence: 0,
            source_revision: 1,
            dependency_revision: 1,
            resolved_anchor: ResolvedReaderAnchor::NotRequested,
        });
        let resources = vec![crate::timeline::ReaderAvatarResource {
            user_id: "@reader:example.org".to_owned(),
            mxc_uri: mxc_uri,
            lease: Err(crate::renderable_thumbnail::ThumbnailLeaseError::Capacity),
        }];

        assert_eq!(
            prepare(model, &ViewBudget::default(), resources).err(),
            Some(ScopeError::Capacity)
        );
    }

    #[test]
    fn compact_admission_preserves_exact_counts_and_rejects_oversized_shapes() {
        let source: ReceiptSourceRef = serde_json::from_value(serde_json::json!({
            "key": {"account_key": "account", "kind": {"Room": {"room_id": "!r:example.org"}}},
            "projection_request_id": {"connection_id": "1", "sequence": "2"},
            "generation": "3", "event_id": "$event"
        }))
        .unwrap();
        let rows: Vec<_> = (0..4)
            .map(|i| ReaderRow {
                user_id: format!("@r{i}:example.org"),
                display_label: "Reader".into(),
                original_display_label: "Reader".into(),
                initials: "R".into(),
                timestamp: None,
                avatar: None,
            })
            .collect();
        let mut model = ViewModel::TimelineReceipts {
            source: source.timeline.clone(),
            summaries: vec![ReceiptCompactSummary {
                source: source.clone(),
                total_count: 1500,
                overflow_count: 1497,
                readers: rows[..3].to_vec(),
            }],
        };
        assert!(valid_shape(&model));
        let ViewModel::TimelineReceipts { summaries, .. } = &mut model else {
            unreachable!()
        };
        summaries[0].readers = rows;
        assert!(!valid_shape(&model));
        let ViewModel::TimelineReceipts { summaries, .. } = &mut model else {
            unreachable!()
        };
        summaries[0].total_count = 4;
        summaries[0].overflow_count = 0;
        assert!(valid_shape(&model));
        let ViewModel::TimelineReceipts { summaries, .. } = &mut model else {
            unreachable!()
        };
        summaries[0].readers[1].user_id = summaries[0].readers[0].user_id.clone();
        assert!(!valid_shape(&model));
        let ViewModel::TimelineReceipts { summaries, .. } = model else {
            unreachable!()
        };
        let mut window = koushi_protocol::view::ReaderWindow {
            source,
            total_count: 1500,
            start: 1499,
            rows: summaries[0].readers[..1].to_vec(),
            window_sequence: 1,
            source_revision: 1,
            dependency_revision: 1,
            resolved_anchor: ResolvedReaderAnchor::NotRequested,
        };
        assert!(valid_shape(&ViewModel::ReaderReady(window.clone())));
        window.start = u64::MAX;
        assert!(!valid_shape(&ViewModel::ReaderReady(window.clone())));
        window.start = 1499;
        window.resolved_anchor = ResolvedReaderAnchor::Row {
            user_id: window.rows[0].user_id.clone(),
            index: 1500,
        };
        assert!(!valid_shape(&ViewModel::ReaderReady(window.clone())));
        window.resolved_anchor = ResolvedReaderAnchor::NotRequested;
        window.start = 0;
        window.rows = (0..257)
            .map(|i| {
                let mut row = window.rows[0].clone();
                row.user_id = format!("@r{i}:example.org");
                row
            })
            .collect();
        assert!(!valid_shape(&ViewModel::ReaderReady(window.clone())));
        window.rows.pop();
        assert!(valid_shape(&ViewModel::ReaderReady(window)));
    }
}
