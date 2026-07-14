use std::{collections::BTreeMap, fmt};

use koushi_media::{
    ImagePreparationPolicy, PreparedImageFormat, PreparedImageVariant, prepare_image_variants,
};
use koushi_state::{
    ComposerTarget, ImageUploadCompressionMode, ImageUploadCompressionPolicy,
    MediaPreparationFailureKind, PreparedUploadFormat, PreparedUploadVariant,
    StagedUploadCompressionChoice, StagedUploadItem, StagedUploadKind, StagedUploadPreparation,
};

pub const MAX_PREPARATION_BATCH_SIZE: usize = 16;

#[derive(Clone)]
pub struct StageUploadBytesInput {
    pub staged_id: String,
    pub position: u64,
    pub filename: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for StageUploadBytesInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StageUploadBytesInput")
            .field("staged_id", &"StagedUploadId(..)")
            .field("position", &self.position)
            .field("filename", &"MediaFilename(..)")
            .field("mime_type", &self.mime_type)
            .field("byte_count", &self.bytes.len())
            .finish()
    }
}

#[derive(Clone)]
struct CachedVariant {
    descriptor: PreparedUploadVariant,
    bytes: Vec<u8>,
}

#[derive(Clone)]
pub struct PreparedMediaUpload {
    pub descriptor: PreparedUploadVariant,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for PreparedMediaUpload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedMediaUpload")
            .field("descriptor", &self.descriptor)
            .field("bytes", &format_args!("{} byte(s)", self.bytes.len()))
            .finish()
    }
}

#[derive(Default)]
pub struct MediaPreparationRegistry {
    variants: BTreeMap<(ComposerTarget, String, String), CachedVariant>,
    selected: BTreeMap<(ComposerTarget, String), String>,
    sources: BTreeMap<(ComposerTarget, String), StageUploadBytesInput>,
}

impl MediaPreparationRegistry {
    pub fn prepare_target(
        &mut self,
        target: &ComposerTarget,
        inputs: Vec<StageUploadBytesInput>,
        mode: ImageUploadCompressionMode,
        policy: ImageUploadCompressionPolicy,
    ) -> Vec<StagedUploadItem> {
        self.clear_target(target);
        self.prepare_items(target, inputs, mode, policy)
    }

    pub fn prepare_items(
        &mut self,
        target: &ComposerTarget,
        inputs: Vec<StageUploadBytesInput>,
        mode: ImageUploadCompressionMode,
        policy: ImageUploadCompressionPolicy,
    ) -> Vec<StagedUploadItem> {
        inputs
            .into_iter()
            .take(MAX_PREPARATION_BATCH_SIZE)
            .map(|input| self.prepare_one(target, input, mode, policy))
            .collect()
    }

    pub fn select_variant(
        &mut self,
        target: &ComposerTarget,
        staged_id: &str,
        variant_id: &str,
    ) -> bool {
        let cache_key = (target.clone(), staged_id.to_owned(), variant_id.to_owned());
        if !self.variants.contains_key(&cache_key) {
            return false;
        }
        self.selected.insert(
            (target.clone(), staged_id.to_owned()),
            variant_id.to_owned(),
        );
        true
    }

    pub fn selected_upload(
        &self,
        target: &ComposerTarget,
        staged_id: &str,
    ) -> Option<PreparedMediaUpload> {
        let selected = self.selected.get(&(target.clone(), staged_id.to_owned()))?;
        let cached =
            self.variants
                .get(&(target.clone(), staged_id.to_owned(), selected.clone()))?;
        Some(PreparedMediaUpload {
            descriptor: cached.descriptor.clone(),
            bytes: cached.bytes.clone(),
        })
    }

    pub fn variant_bytes(
        &self,
        target: &ComposerTarget,
        staged_id: &str,
        variant_id: &str,
    ) -> Option<Vec<u8>> {
        self.variants
            .get(&(target.clone(), staged_id.to_owned(), variant_id.to_owned()))
            .map(|cached| cached.bytes.clone())
    }

    pub fn remove_item(&mut self, target: &ComposerTarget, staged_id: &str) {
        self.variants
            .retain(|(item_target, item_id, _), _| item_target != target || item_id != staged_id);
        self.selected
            .remove(&(target.clone(), staged_id.to_owned()));
        self.sources.remove(&(target.clone(), staged_id.to_owned()));
    }

    pub fn clear_target(&mut self, target: &ComposerTarget) {
        self.variants
            .retain(|(item_target, _, _), _| item_target != target);
        self.selected
            .retain(|(item_target, _), _| item_target != target);
        self.sources
            .retain(|(item_target, _), _| item_target != target);
    }

    pub fn clear(&mut self) {
        self.variants.clear();
        self.selected.clear();
        self.sources.clear();
    }

    pub fn retry_item(
        &mut self,
        target: &ComposerTarget,
        staged_id: &str,
        mode: ImageUploadCompressionMode,
        policy: ImageUploadCompressionPolicy,
    ) -> Option<StagedUploadItem> {
        let input = self
            .sources
            .get(&(target.clone(), staged_id.to_owned()))?
            .clone();
        self.remove_prepared_variants(target, staged_id);
        Some(self.prepare_one(target, input, mode, policy))
    }

    pub fn use_original(
        &mut self,
        target: &ComposerTarget,
        staged_id: &str,
    ) -> Option<StagedUploadItem> {
        let input = self
            .sources
            .get(&(target.clone(), staged_id.to_owned()))?
            .clone();
        if input.bytes.is_empty() {
            return None;
        }
        let byte_count = u64::try_from(input.bytes.len()).unwrap_or(u64::MAX);
        self.remove_prepared_variants(target, staged_id);
        Some(self.store_original_file(target, input, byte_count))
    }

    fn remove_prepared_variants(&mut self, target: &ComposerTarget, staged_id: &str) {
        self.variants
            .retain(|(item_target, item_id, _), _| item_target != target || item_id != staged_id);
        self.selected
            .remove(&(target.clone(), staged_id.to_owned()));
    }

    fn prepare_one(
        &mut self,
        target: &ComposerTarget,
        input: StageUploadBytesInput,
        mode: ImageUploadCompressionMode,
        policy: ImageUploadCompressionPolicy,
    ) -> StagedUploadItem {
        self.sources
            .insert((target.clone(), input.staged_id.clone()), input.clone());
        let byte_count = u64::try_from(input.bytes.len()).unwrap_or(u64::MAX);
        let image_candidate = matches!(
            input.mime_type.to_ascii_lowercase().as_str(),
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        );
        if input.bytes.is_empty() {
            return staged_failure(
                target,
                input,
                byte_count,
                MediaPreparationFailureKind::Empty,
            );
        }

        if !image_candidate {
            return self.store_original_file(target, input, byte_count);
        }

        let prepared = prepare_image_variants(
            &input.bytes,
            &input.filename,
            &input.mime_type,
            &ImagePreparationPolicy {
                target_long_edge: u32::try_from(policy.target_long_edge).unwrap_or(u32::MAX),
                quality_percent: policy.quality_percent,
            },
        );
        let variants = match prepared {
            Ok(variants) => variants,
            Err(_) => {
                return staged_failure(
                    target,
                    input,
                    byte_count,
                    MediaPreparationFailureKind::Encode,
                );
            }
        };
        let original_len = input.bytes.len();
        let selected_variant_id = select_initial_variant(&variants, mode);
        let descriptors = variants
            .into_iter()
            .map(|variant| {
                let descriptor = descriptor_from_image_variant(&variant, original_len);
                self.variants.insert(
                    (
                        target.clone(),
                        input.staged_id.clone(),
                        descriptor.variant_id.clone(),
                    ),
                    CachedVariant {
                        descriptor: descriptor.clone(),
                        bytes: variant.bytes,
                    },
                );
                descriptor
            })
            .collect::<Vec<_>>();
        self.selected.insert(
            (target.clone(), input.staged_id.clone()),
            selected_variant_id.clone(),
        );
        let selected = descriptors
            .iter()
            .find(|variant| variant.variant_id == selected_variant_id)
            .expect("selected prepared variant exists");
        StagedUploadItem {
            staged_id: input.staged_id,
            room_id: target.room_id().to_owned(),
            position: input.position,
            filename: selected.filename.clone(),
            mime_type: selected.mime_type.clone(),
            byte_count: selected.byte_count,
            kind: StagedUploadKind::Image {
                width: selected.width,
                height: selected.height,
            },
            caption: None,
            compression_choice: StagedUploadCompressionChoice::Original,
            preparation: StagedUploadPreparation::Ready {
                variants: descriptors,
                selected_variant_id,
            },
        }
    }

    fn store_original_file(
        &mut self,
        target: &ComposerTarget,
        input: StageUploadBytesInput,
        byte_count: u64,
    ) -> StagedUploadItem {
        let descriptor = PreparedUploadVariant {
            variant_id: "original".to_owned(),
            filename: input.filename.clone(),
            mime_type: normalized_mime(&input.mime_type),
            byte_count,
            width: None,
            height: None,
            format: PreparedUploadFormat::Original,
            savings_percent: 0,
            metadata_stripped: false,
            thumbnail_refreshed: false,
        };
        self.variants.insert(
            (
                target.clone(),
                input.staged_id.clone(),
                descriptor.variant_id.clone(),
            ),
            CachedVariant {
                descriptor: descriptor.clone(),
                bytes: input.bytes,
            },
        );
        self.selected.insert(
            (target.clone(), input.staged_id.clone()),
            descriptor.variant_id.clone(),
        );
        StagedUploadItem {
            staged_id: input.staged_id,
            room_id: target.room_id().to_owned(),
            position: input.position,
            filename: descriptor.filename.clone(),
            mime_type: descriptor.mime_type.clone(),
            byte_count,
            kind: StagedUploadKind::File,
            caption: None,
            compression_choice: StagedUploadCompressionChoice::NotApplicable,
            preparation: StagedUploadPreparation::Ready {
                variants: vec![descriptor],
                selected_variant_id: "original".to_owned(),
            },
        }
    }
}

impl fmt::Debug for MediaPreparationRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaPreparationRegistry")
            .field("variant_count", &self.variants.len())
            .field("selected_count", &self.selected.len())
            .finish()
    }
}

fn staged_failure(
    target: &ComposerTarget,
    input: StageUploadBytesInput,
    byte_count: u64,
    failure_kind: MediaPreparationFailureKind,
) -> StagedUploadItem {
    StagedUploadItem {
        staged_id: input.staged_id,
        room_id: target.room_id().to_owned(),
        position: input.position,
        filename: input.filename,
        mime_type: normalized_mime(&input.mime_type),
        byte_count,
        kind: StagedUploadKind::File,
        caption: None,
        compression_choice: StagedUploadCompressionChoice::NotApplicable,
        preparation: StagedUploadPreparation::Failed {
            failure_kind,
            can_use_original: !input.bytes.is_empty(),
        },
    }
}

fn descriptor_from_image_variant(
    variant: &PreparedImageVariant,
    original_len: usize,
) -> PreparedUploadVariant {
    let byte_count = u64::try_from(variant.bytes.len()).unwrap_or(u64::MAX);
    let savings_percent = if original_len == 0 {
        0
    } else {
        100 - i64::try_from(variant.bytes.len().saturating_mul(100) / original_len).unwrap_or(100)
    };
    PreparedUploadVariant {
        variant_id: variant.id.clone(),
        filename: variant.filename.clone(),
        mime_type: variant.mime_type.clone(),
        byte_count,
        width: Some(u64::from(variant.dimensions.0)),
        height: Some(u64::from(variant.dimensions.1)),
        format: match variant.format {
            PreparedImageFormat::Png => PreparedUploadFormat::Png,
            PreparedImageFormat::Jpeg => PreparedUploadFormat::Jpeg,
            PreparedImageFormat::WebP => PreparedUploadFormat::Webp,
            PreparedImageFormat::Gif | PreparedImageFormat::Other => PreparedUploadFormat::Original,
        },
        savings_percent,
        metadata_stripped: variant.metadata_stripped,
        thumbnail_refreshed: variant.thumbnail_refreshed,
    }
}

fn select_initial_variant(
    variants: &[PreparedImageVariant],
    mode: ImageUploadCompressionMode,
) -> String {
    if mode == ImageUploadCompressionMode::Never {
        return "original".to_owned();
    }
    variants
        .iter()
        .find(|variant| variant.recommended)
        .or_else(|| variants.first())
        .map(|variant| variant.id.clone())
        .unwrap_or_else(|| "original".to_owned())
}

fn normalized_mime(mime_type: &str) -> String {
    let mime_type = mime_type.trim();
    if mime_type.is_empty() {
        "application/octet-stream".to_owned()
    } else {
        mime_type.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(root: Option<&str>) -> ComposerTarget {
        match root {
            Some(root_event_id) => ComposerTarget::Thread {
                room_id: "!room:example.invalid".to_owned(),
                root_event_id: root_event_id.to_owned(),
            },
            None => ComposerTarget::Main {
                room_id: "!room:example.invalid".to_owned(),
            },
        }
    }

    fn file(id: &str, bytes: &[u8]) -> StageUploadBytesInput {
        StageUploadBytesInput {
            staged_id: id.to_owned(),
            position: 1,
            filename: "private.pdf".to_owned(),
            mime_type: "application/pdf".to_owned(),
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn registry_isolates_equal_ids_by_target_and_clears_bytes() {
        let mut registry = MediaPreparationRegistry::default();
        let main = target(None);
        let thread = target(Some("$root"));
        let policy = ImageUploadCompressionPolicy::default();
        registry.prepare_target(
            &main,
            vec![file("shared", b"main")],
            ImageUploadCompressionMode::Never,
            policy,
        );
        registry.prepare_target(
            &thread,
            vec![file("shared", b"thread")],
            ImageUploadCompressionMode::Never,
            policy,
        );

        assert_eq!(
            registry.selected_upload(&main, "shared").unwrap().bytes,
            b"main"
        );
        assert_eq!(
            registry.selected_upload(&thread, "shared").unwrap().bytes,
            b"thread"
        );
        registry.clear_target(&thread);
        assert!(registry.selected_upload(&thread, "shared").is_none());
        assert_eq!(
            registry.selected_upload(&main, "shared").unwrap().bytes,
            b"main"
        );
    }

    #[test]
    fn empty_input_is_a_typed_failure_and_debug_is_private() {
        let mut registry = MediaPreparationRegistry::default();
        let target = target(None);
        let items = registry.prepare_target(
            &target,
            vec![file("private-stage", b"")],
            ImageUploadCompressionMode::Ask,
            ImageUploadCompressionPolicy::default(),
        );
        assert!(matches!(
            items[0].preparation,
            StagedUploadPreparation::Failed {
                failure_kind: MediaPreparationFailureKind::Empty,
                ..
            }
        ));
        let debug = format!("{:?}", file("private-stage", b"private bytes"));
        assert!(!debug.contains("private.pdf"));
        assert!(!debug.contains("private bytes"));
    }

    #[test]
    fn failed_item_can_promote_its_captured_nonempty_source_to_original() {
        let mut registry = MediaPreparationRegistry::default();
        let target = target(Some("$root"));
        registry.sources.insert(
            (target.clone(), "failed".to_owned()),
            StageUploadBytesInput {
                staged_id: "failed".to_owned(),
                position: 2,
                filename: "private.bin".to_owned(),
                mime_type: "application/octet-stream".to_owned(),
                bytes: b"captured source".to_vec(),
            },
        );

        let item = registry
            .use_original(&target, "failed")
            .expect("captured original should be selectable");
        assert!(matches!(
            item.preparation,
            StagedUploadPreparation::Ready { .. }
        ));
        assert_eq!(
            registry.selected_upload(&target, "failed").unwrap().bytes,
            b"captured source"
        );
    }
}
