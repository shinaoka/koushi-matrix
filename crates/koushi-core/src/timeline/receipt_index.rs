use std::{
    cmp::Reverse,
    sync::{Arc, Mutex, Weak},
};

use eyeball_im::Vector;
#[cfg(test)]
use koushi_protocol::view::{ReaderWindowLimit, ResolvedReaderAnchor};
use matrix_sdk::ruma::{OwnedUserId, UserId, events::receipt::Receipt};
use matrix_sdk_ui::timeline::ReadReceiptSnapshot;

type ReaderKey = (Reverse<u64>, OwnedUserId);

static NEXT_RECEIPT_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_source_revision(counter: &std::sync::atomic::AtomicU64) -> Option<u64> {
    use std::sync::atomic::Ordering;
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .ok()
        .map(|previous| previous + 1)
}

pub(crate) struct ReceiptEpoch {
    pub(crate) valid: bool,
    pub(crate) revision: Option<u64>,
}

impl ReceiptEpoch {
    fn new() -> Self {
        let revision = next_source_revision(&NEXT_RECEIPT_REVISION);
        Self {
            valid: revision.is_some(),
            revision,
        }
    }
}

/// Single-owner derived order; published rows never share this vector.
pub(super) struct ReceiptReaderIndex {
    receipts: ReadReceiptSnapshot,
    order: Vector<ReaderKey>,
    window_epoch: Arc<Mutex<ReceiptEpoch>>,
}

impl ReceiptReaderIndex {
    pub(super) fn new(receipts: ReadReceiptSnapshot) -> Self {
        let mut keys: Vec<_> = receipts
            .iter()
            .map(|(user, receipt)| reader_key(user, receipt))
            .collect();
        keys.sort_unstable();
        Self {
            receipts,
            order: keys.into_iter().collect(),
            window_epoch: Arc::new(Mutex::new(ReceiptEpoch::new())),
        }
    }

    #[cfg(test)]
    fn source_revision(&self) -> Option<u64> {
        self.window_epoch
            .lock()
            .expect("receipt epoch poisoned")
            .revision
    }

    pub(super) fn window_epoch(&self) -> Weak<Mutex<ReceiptEpoch>> {
        Arc::downgrade(&self.window_epoch)
    }

    pub(super) fn update(&mut self, after: ReadReceiptSnapshot) {
        let mut changed = false;
        for (user, replacement) in after.changes_since(&self.receipts) {
            if !changed {
                self.window_epoch
                    .lock()
                    .expect("receipt epoch poisoned")
                    .valid = false;
                changed = true;
            }
            let previous = self.receipts.get(user);
            if previous.map(sort_timestamp) == replacement.map(sort_timestamp) {
                continue;
            }
            if let Some(previous) = previous {
                let position = self
                    .order
                    .binary_search(&reader_key(user, previous))
                    .expect("existing receipt has an ordered key");
                self.order.remove(position);
            }
            if let Some(replacement) = replacement {
                let key = reader_key(user, replacement);
                let position = self
                    .order
                    .binary_search(&key)
                    .expect_err("new receipt key is not already indexed");
                self.order.insert(position, key);
            }
        }
        if changed {
            self.window_epoch = Arc::new(Mutex::new(ReceiptEpoch::new()));
        }
        self.receipts = after;
    }

    pub(super) fn total(&self, own_user: Option<&UserId>) -> usize {
        self.order.len()
            - usize::from(own_user.is_some_and(|user| self.receipts.get(user).is_some()))
    }

    /// Reject anchors outside the installed window; never substitute newly arrived readers.
    #[cfg(test)]
    pub(super) fn resolve_anchor(
        &self,
        installed: &[OwnedUserId],
        anchor: &UserId,
        own_user: Option<&UserId>,
    ) -> Option<ResolvedReaderAnchor> {
        ReaderWindowLimit::try_from(u16::try_from(installed.len()).ok()?).ok()?;
        let position = installed
            .iter()
            .position(|user| user.as_str() == anchor.as_str())?;
        let candidates = std::iter::once(&installed[position])
            .chain(&installed[position + 1..])
            .chain(installed[..position].iter().rev());
        for user in candidates {
            if let Some(index) = self.rank(user, own_user) {
                return Some(ResolvedReaderAnchor::Row {
                    user_id: user.to_string(),
                    index: index as u64,
                });
            }
        }
        Some(ResolvedReaderAnchor::NoSurvivingInstalledRow)
    }

    #[cfg(test)]
    fn rank(&self, user: &UserId, own_user: Option<&UserId>) -> Option<usize> {
        if own_user == Some(user) {
            return None;
        }
        let key = reader_key(user, self.receipts.get(user)?);
        let position = self
            .order
            .binary_search(&key)
            .expect("receipt has an ordered key");
        let own_before = own_user.is_some_and(|own| {
            self.receipts
                .get(own)
                .is_some_and(|receipt| reader_key(own, receipt) < key)
        });
        Some(position - usize::from(own_before))
    }

    pub(super) fn window(
        &self,
        start: usize,
        limit: usize,
        own_user: Option<&UserId>,
    ) -> impl ExactSizeIterator<Item = (&OwnedUserId, &Receipt)> {
        let own_position = own_user.and_then(|user| {
            self.receipts.get(user).map(|receipt| {
                self.order
                    .binary_search(&reader_key(user, receipt))
                    .expect("own receipt has an ordered key")
            })
        });
        let total = self.order.len() - usize::from(own_position.is_some());
        let start = start.min(total);
        let end = start.saturating_add(limit).min(total);
        (start..end).map(move |index| {
            let source_index = index + usize::from(own_position.is_some_and(|own| index >= own));
            let user = &self.order[source_index].1;
            (
                user,
                self.receipts
                    .get(user)
                    .expect("ordered reader has a receipt"),
            )
        })
    }
}

impl Drop for ReceiptReaderIndex {
    fn drop(&mut self) {
        self.window_epoch
            .lock()
            .expect("receipt epoch poisoned")
            .valid = false;
    }
}

fn sort_timestamp(receipt: &Receipt) -> u64 {
    receipt
        .ts
        .map(|timestamp| timestamp.0.into())
        .unwrap_or_default()
}

fn reader_key(user: &UserId, receipt: &Receipt) -> ReaderKey {
    (Reverse(sort_timestamp(receipt)), user.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_sdk::ruma::{MilliSecondsSinceUnixEpoch, user_id};

    fn snapshot(values: &[(&str, Option<u32>)]) -> ReadReceiptSnapshot {
        values
            .iter()
            .map(|(user, timestamp)| {
                let mut receipt = Receipt::default();
                receipt.ts = timestamp.map(|value| MilliSecondsSinceUnixEpoch(value.into()));
                (user.parse().unwrap(), receipt)
            })
            .collect()
    }

    #[test]
    fn receipt_revisions_advance_across_changes_and_replacement_without_wrapping() {
        let mut index = ReceiptReaderIndex::new(snapshot(&[("@a:example.org", None)]));
        let first = index.source_revision().unwrap();
        index.update(snapshot(&[("@a:example.org", None)]));
        assert_eq!(index.source_revision(), Some(first));
        index.update(snapshot(&[("@a:example.org", Some(0))]));
        let changed = index.source_revision().unwrap();
        assert!(changed > first);
        let replacement = ReceiptReaderIndex::new(snapshot(&[]));
        assert!(replacement.source_revision().unwrap() > changed);
        let exhausted = std::sync::atomic::AtomicU64::new(u64::MAX);
        assert_eq!(next_source_revision(&exhausted), None);
        assert_eq!(
            exhausted.load(std::sync::atomic::Ordering::Relaxed),
            u64::MAX
        );
    }

    #[test]
    fn prepared_window_expires_on_change_or_removal_but_not_noop() {
        let mut index = ReceiptReaderIndex::new(snapshot(&[("@a:example.org", None)]));
        let first = index.window_epoch();
        index.update(snapshot(&[("@a:example.org", None)]));
        assert_eq!(first.strong_count(), 1);
        // None and zero sort equally, but the returned timestamp changed.
        index.update(snapshot(&[("@a:example.org", Some(0))]));
        assert_eq!(first.strong_count(), 0);
        let current = index.window_epoch();
        drop(index);
        assert_eq!(current.strong_count(), 0);
    }

    #[test]
    fn anchor_resolution_uses_only_installed_survivors() {
        let own = user_id!("@self:example.org");
        let mut index = ReceiptReaderIndex::new(snapshot(&[
            ("@a:example.org", Some(40)),
            ("@b:example.org", Some(30)),
            ("@c:example.org", Some(20)),
            ("@d:example.org", Some(10)),
        ]));
        let installed: Vec<_> = index
            .window(0, 4, Some(own))
            .map(|(user, _)| user.clone())
            .collect();
        let anchor = user_id!("@b:example.org");
        let resolved = |user: &str, index| {
            Some(ResolvedReaderAnchor::Row {
                user_id: user.into(),
                index,
            })
        };
        assert_eq!(
            index.resolve_anchor(&installed, anchor, Some(own)),
            resolved(anchor.as_str(), 1)
        );
        index.update(snapshot(&[
            ("@new:example.org", Some(100)),
            ("@self:example.org", Some(90)),
            ("@d:example.org", Some(80)),
            ("@a:example.org", Some(40)),
            ("@c:example.org", Some(20)),
        ]));
        // Installed successor c wins over d even though d now sorts before it.
        assert_eq!(
            index.resolve_anchor(&installed, anchor, Some(own)),
            resolved("@c:example.org", 3)
        );
        index.update(snapshot(&[
            ("@new:example.org", Some(100)),
            ("@a:example.org", Some(40)),
        ]));
        assert_eq!(
            index.resolve_anchor(&installed, anchor, Some(own)),
            resolved("@a:example.org", 1)
        );
        index.update(snapshot(&[
            ("@new:example.org", Some(100)),
            ("@a:example.org", Some(40)),
            ("@b:example.org", Some(30)),
        ]));
        assert_eq!(
            index.resolve_anchor(&installed, user_id!("@c:example.org"), Some(own)),
            resolved("@b:example.org", 2)
        );
        let oversized = vec![anchor.to_owned(); 257];
        assert_eq!(index.resolve_anchor(&oversized, anchor, Some(own)), None);
        index.update(snapshot(&[("@new:example.org", Some(100))]));
        assert_eq!(
            index.resolve_anchor(&installed, anchor, Some(own)),
            Some(ResolvedReaderAnchor::NoSurvivingInstalledRow)
        );
        assert_eq!(
            index.resolve_anchor(&installed, user_id!("@new:example.org"), Some(own)),
            None
        );
    }

    #[test]
    fn updating_one_snapshot_with_many_receipts_indexes_every_reader() {
        let mut index = ReceiptReaderIndex::new(snapshot(&[("@first:example.org", Some(1))]));
        index.update(snapshot(&[
            ("@first:example.org", Some(1)),
            ("@second:example.org", Some(2)),
            ("@third:example.org", Some(3)),
            ("@fourth:example.org", Some(4)),
            ("@fifth:example.org", Some(5)),
        ]));

        assert_eq!(index.total(None), 5);
        assert_eq!(index.window(0, usize::MAX, None).count(), 5);
    }

    #[test]
    fn ordered_windows_preserve_totals_own_exclusion_and_updates() {
        let own = user_id!("@self:example.org");
        let before = snapshot(&[
            ("@self:example.org", Some(50)),
            ("@b:example.org", Some(50)),
            ("@m:example.org", None),
            ("@a:example.org", Some(0)),
        ]);
        let mut index = ReceiptReaderIndex::new(before.clone());
        let ids = |index: &ReceiptReaderIndex, start, limit| {
            index
                .window(start, limit, Some(own))
                .map(|(user, _)| user.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(index.total(Some(own)), 3);
        assert_eq!(
            ids(&index, 0, 10),
            ["@b:example.org", "@a:example.org", "@m:example.org"]
        );
        assert_eq!(ids(&index, 1, 1), ["@a:example.org"]);
        assert!(ids(&index, usize::MAX, 10).is_empty());
        index.update(snapshot(&[
            ("@self:example.org", Some(50)),
            ("@m:example.org", Some(0)),
            ("@a:example.org", Some(75)),
            ("@c:example.org", Some(40)),
        ]));
        assert_eq!(index.total(Some(own)), 3);
        assert_eq!(
            ids(&index, 0, 10),
            ["@a:example.org", "@c:example.org", "@m:example.org"]
        );
        assert_eq!(index.total(None), 4);
        assert_eq!(
            index.window(2, 1, Some(own)).next().unwrap().1.ts,
            Some(MilliSecondsSinceUnixEpoch(0_u32.into()))
        );
        assert_eq!(before.len(), 4);
        index.update(ReadReceiptSnapshot::default());
        assert_eq!(index.total(Some(own)), 0);
        assert!(ids(&index, 0, 10).is_empty());
    }
}
