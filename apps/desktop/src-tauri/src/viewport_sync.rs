use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{Runtime, WebviewWindow};

pub(crate) const VIEWPORT_TOLERANCE_POINTS: f64 = 0.5;
const MAX_VIEWPORT_DIMENSION: f64 = 1_000_000.0;
const DIAGNOSTIC_SOURCE: &str = "desktop.viewport_sync";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ViewportSyncTrigger {
    PageLoad,
    Resized,
    ScaleFactorChanged,
    DensityCommit,
    BrowserResize,
    Moved,
    Panel,
}

impl ViewportSyncTrigger {
    pub(crate) const fn is_admitted(self) -> bool {
        matches!(
            self,
            Self::PageLoad
                | Self::Resized
                | Self::ScaleFactorChanged
                | Self::DensityCommit
                | Self::BrowserResize
        )
    }

    pub(crate) const fn token(self) -> &'static str {
        match self {
            Self::PageLoad => "page_load",
            Self::Resized => "resized",
            Self::ScaleFactorChanged => "scale_factor_changed",
            Self::DensityCommit => "density_commit",
            Self::BrowserResize => "browser_resize",
            Self::Moved => "moved",
            Self::Panel => "panel",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ViewportDensity {
    Compact,
    Default,
    Comfortable,
}

impl ViewportDensity {
    pub(crate) const fn token(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Default => "default",
            Self::Comfortable => "comfortable",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ViewportSize {
    pub(crate) width: f64,
    pub(crate) height: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ViewportRect {
    pub(crate) top: f64,
    pub(crate) left: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VisualViewportObservation {
    pub(crate) present: bool,
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) offset_left: f64,
    pub(crate) offset_top: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ViewportSyncObservation {
    pub(crate) trigger: ViewportSyncTrigger,
    pub(crate) density: ViewportDensity,
    pub(crate) window: ViewportSize,
    pub(crate) document: ViewportSize,
    pub(crate) visual_viewport: VisualViewportObservation,
    pub(crate) body: ViewportRect,
    pub(crate) root: ViewportRect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ViewportSyncValidationError {
    TriggerNotAdmitted,
    NonFiniteMeasurement,
    InvalidMeasurement,
}

impl std::fmt::Display for ViewportSyncValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::TriggerNotAdmitted => "viewport trigger is not admitted",
            Self::NonFiniteMeasurement => "viewport measurement was not finite",
            Self::InvalidMeasurement => "viewport measurement was incomplete or invalid",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ViewportSyncValidationError {}

impl ViewportSize {
    fn is_finite(self) -> bool {
        self.width.is_finite() && self.height.is_finite()
    }

    fn is_valid(self) -> bool {
        self.is_finite()
            && self.width > 0.0
            && self.height > 0.0
            && self.width <= MAX_VIEWPORT_DIMENSION
            && self.height <= MAX_VIEWPORT_DIMENSION
    }
}

impl ViewportRect {
    fn is_finite(self) -> bool {
        self.top.is_finite()
            && self.left.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
    }

    fn is_valid(self) -> bool {
        self.is_finite()
            && self.width > 0.0
            && self.height > 0.0
            && self.width <= MAX_VIEWPORT_DIMENSION
            && self.height <= MAX_VIEWPORT_DIMENSION
    }
}

impl VisualViewportObservation {
    fn is_finite(self) -> bool {
        self.width.is_finite()
            && self.height.is_finite()
            && self.offset_left.is_finite()
            && self.offset_top.is_finite()
    }

    fn is_valid(self) -> bool {
        self.is_finite()
            && if self.present {
                self.width > 0.0
                    && self.height > 0.0
                    && self.width <= MAX_VIEWPORT_DIMENSION
                    && self.height <= MAX_VIEWPORT_DIMENSION
            } else {
                self.width == 0.0
                    && self.height == 0.0
                    && self.offset_left == 0.0
                    && self.offset_top == 0.0
            }
    }
}

pub(crate) fn validate_observation(
    observation: &ViewportSyncObservation,
) -> Result<(), ViewportSyncValidationError> {
    if !observation.trigger.is_admitted() {
        return Err(ViewportSyncValidationError::TriggerNotAdmitted);
    }
    if !observation.window.is_finite()
        || !observation.document.is_finite()
        || !observation.visual_viewport.is_finite()
        || !observation.body.is_finite()
        || !observation.root.is_finite()
    {
        return Err(ViewportSyncValidationError::NonFiniteMeasurement);
    }
    if !observation.window.is_valid()
        || !observation.document.is_valid()
        || !observation.visual_viewport.is_valid()
        || !observation.body.is_valid()
        || !observation.root.is_valid()
    {
        return Err(ViewportSyncValidationError::InvalidMeasurement);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NativeViewportSupport {
    Supported,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ViewportSyncDecision {
    InSync,
    RepairToParentBounds,
    Unsupported,
}

pub(crate) fn rects_align(left: ViewportRect, right: ViewportRect) -> bool {
    approx_equal(left.top, right.top)
        && approx_equal(left.left, right.left)
        && approx_equal(left.width, right.width)
        && approx_equal(left.height, right.height)
}

pub(crate) fn viewport_sync_decision(
    parent_bounds: ViewportRect,
    webview_frame: ViewportRect,
) -> ViewportSyncDecision {
    if rects_align(parent_bounds, webview_frame) {
        ViewportSyncDecision::InSync
    } else {
        ViewportSyncDecision::RepairToParentBounds
    }
}

fn dom_root_is_aligned(observation: &ViewportSyncObservation) -> bool {
    let document_bounds = ViewportRect {
        top: 0.0,
        left: 0.0,
        width: observation.document.width,
        height: observation.document.height,
    };
    rects_align(observation.root, document_bounds) && rects_align(observation.body, document_bounds)
}

fn dom_js_is_aligned(observation: &ViewportSyncObservation) -> bool {
    approx_equal(observation.window.width, observation.document.width)
        && approx_equal(observation.window.height, observation.document.height)
        && (!observation.visual_viewport.present
            || (approx_equal(observation.visual_viewport.width, observation.window.width)
                && approx_equal(
                    observation.visual_viewport.height,
                    observation.window.height,
                )
                && approx_zero(observation.visual_viewport.offset_left)
                && approx_zero(observation.visual_viewport.offset_top)))
}

pub(crate) fn dom_is_aligned(observation: &ViewportSyncObservation) -> bool {
    dom_js_is_aligned(observation) && dom_root_is_aligned(observation)
}

fn approx_equal(left: f64, right: f64) -> bool {
    (left - right).abs() <= VIEWPORT_TOLERANCE_POINTS
}

fn approx_zero(value: f64) -> bool {
    value.abs() <= VIEWPORT_TOLERANCE_POINTS
}

#[derive(Debug, Default)]
pub(crate) struct ViewportSyncGeneration(AtomicU64);

impl ViewportSyncGeneration {
    pub(crate) fn next(&self) -> Result<u64, &'static str> {
        let mut current = self.0.load(Ordering::Relaxed);
        loop {
            let next = current
                .checked_add(1)
                .ok_or("viewport observation generation exhausted")?;
            match self
                .0
                .compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return Ok(next),
                Err(observed) => current = observed,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ViewportSyncReceipt {
    pub(crate) generation: u64,
    pub(crate) trigger: ViewportSyncTrigger,
    pub(crate) density: Option<ViewportDensity>,
    pub(crate) native_support: NativeViewportSupport,
    pub(crate) decision: ViewportSyncDecision,
    pub(crate) native_aligned: bool,
    pub(crate) native_origin_aligned: bool,
    pub(crate) native_size_aligned: bool,
    pub(crate) dom_aligned: bool,
    pub(crate) dom_js_aligned: bool,
    pub(crate) dom_root_aligned: bool,
    pub(crate) parent: Option<ViewportRect>,
    pub(crate) webview: Option<ViewportRect>,
}

#[derive(Clone, Copy, Debug)]
struct NativeSyncResult {
    support: NativeViewportSupport,
    decision: ViewportSyncDecision,
    native_aligned: bool,
    native_origin_aligned: bool,
    native_size_aligned: bool,
    parent: Option<ViewportRect>,
    webview: Option<ViewportRect>,
}

impl NativeSyncResult {
    fn unsupported() -> Self {
        Self {
            support: NativeViewportSupport::Unsupported,
            decision: ViewportSyncDecision::Unsupported,
            native_aligned: false,
            native_origin_aligned: false,
            native_size_aligned: false,
            parent: None,
            webview: None,
        }
    }
}

impl ViewportSyncReceipt {
    fn from_native(
        generation: u64,
        trigger: ViewportSyncTrigger,
        native: NativeSyncResult,
    ) -> Self {
        Self {
            generation,
            trigger,
            density: None,
            native_support: native.support,
            decision: native.decision,
            native_aligned: native.native_aligned,
            native_origin_aligned: native.native_origin_aligned,
            native_size_aligned: native.native_size_aligned,
            dom_aligned: false,
            dom_js_aligned: false,
            dom_root_aligned: false,
            parent: native.parent,
            webview: native.webview,
        }
    }

    pub(crate) fn with_dom_observation(mut self, observation: &ViewportSyncObservation) -> Self {
        self.density = Some(observation.density);
        self.dom_js_aligned = dom_js_is_aligned(observation);
        self.dom_root_aligned = dom_root_is_aligned(observation);
        self.dom_aligned = self.dom_js_aligned && self.dom_root_aligned;
        self
    }
}

pub(crate) async fn synchronize_now<R: Runtime>(
    window: WebviewWindow<R>,
    generation: &ViewportSyncGeneration,
    trigger: ViewportSyncTrigger,
) -> Result<ViewportSyncReceipt, String> {
    if !trigger.is_admitted() {
        return Err(ViewportSyncValidationError::TriggerNotAdmitted.to_string());
    }
    let observation_generation = generation.next().map_err(str::to_owned)?;

    #[cfg(target_os = "macos")]
    {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let window_on_main_thread = window.clone();
        window
            .run_on_main_thread(move || {
                let _ = window_on_main_thread.with_webview(move |platform_webview| {
                    // The parent lookup, measurement, policy decision, frame write, and
                    // verification all happen in this one main-thread callback.
                    let native = unsafe { synchronize_macos_webview(platform_webview, trigger) };
                    let _ = sender.send(native);
                });
            })
            .map_err(|_| "viewport main-thread dispatch failed".to_owned())?;
        let native = receiver
            .await
            .map_err(|_| "viewport main-thread result was dropped".to_owned())?;
        Ok(ViewportSyncReceipt::from_native(
            observation_generation,
            trigger,
            native,
        ))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Ok(ViewportSyncReceipt::from_native(
            observation_generation,
            trigger,
            NativeSyncResult::unsupported(),
        ))
    }
}

pub(crate) async fn synchronize_and_record<R: Runtime>(
    window: WebviewWindow<R>,
    generation: &ViewportSyncGeneration,
    trigger: ViewportSyncTrigger,
) -> Result<ViewportSyncReceipt, String> {
    let receipt = synchronize_now(window, generation, trigger).await?;
    record_diagnostic(&receipt, None);
    Ok(receipt)
}

pub(crate) fn record_diagnostic(
    receipt: &ViewportSyncReceipt,
    observation: Option<&ViewportSyncObservation>,
) {
    use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};

    let mut event = DiagnosticEvent::new(DiagnosticLevel::Info, DIAGNOSTIC_SOURCE, "observed")
        .field(DiagnosticField::correlation(
            "generation",
            receipt.generation,
        ))
        .field(DiagnosticField::token("trigger", receipt.trigger.token()))
        .field(DiagnosticField::token(
            "density",
            receipt.density.map_or("none", ViewportDensity::token),
        ))
        .field(DiagnosticField::token(
            "native_support",
            match receipt.native_support {
                NativeViewportSupport::Supported => "supported",
                NativeViewportSupport::Unsupported => "unsupported",
            },
        ))
        .field(DiagnosticField::token(
            "decision",
            match receipt.decision {
                ViewportSyncDecision::InSync => "in_sync",
                ViewportSyncDecision::RepairToParentBounds => "repair_to_parent_bounds",
                ViewportSyncDecision::Unsupported => "unsupported",
            },
        ))
        .field(DiagnosticField::boolean(
            "native_aligned",
            receipt.native_aligned,
        ))
        .field(DiagnosticField::boolean(
            "native_origin_aligned",
            receipt.native_origin_aligned,
        ))
        .field(DiagnosticField::boolean(
            "native_size_aligned",
            receipt.native_size_aligned,
        ))
        .field(DiagnosticField::boolean("dom_aligned", receipt.dom_aligned))
        .field(DiagnosticField::boolean(
            "dom_js_aligned",
            receipt.dom_js_aligned,
        ))
        .field(DiagnosticField::boolean(
            "dom_root_aligned",
            receipt.dom_root_aligned,
        ));

    event = append_rect_dimensions(event, "parent_width", "parent_height", receipt.parent);
    event = append_rect_dimensions(event, "webview_width", "webview_height", receipt.webview);

    if let Some(observation) = observation {
        event = event
            .field(DiagnosticField::boolean(
                "visual_viewport_present",
                observation.visual_viewport.present,
            ))
            .field(DiagnosticField::token(
                "visual_offset_left",
                offset_class(observation.visual_viewport.offset_left),
            ))
            .field(DiagnosticField::token(
                "visual_offset_top",
                offset_class(observation.visual_viewport.offset_top),
            ))
            .field(DiagnosticField::boolean(
                "root_origin_aligned",
                approx_zero(observation.root.top) && approx_zero(observation.root.left),
            ))
            .field(DiagnosticField::boolean(
                "body_origin_aligned",
                approx_zero(observation.body.top) && approx_zero(observation.body.left),
            ));
        event = append_size_dimensions(
            event,
            "window_width",
            "window_height",
            Some(observation.window),
        );
        event = append_size_dimensions(
            event,
            "document_width",
            "document_height",
            Some(observation.document),
        );
        event = append_rect_dimensions(event, "root_width", "root_height", Some(observation.root));
        event = append_rect_dimensions(event, "body_width", "body_height", Some(observation.body));
        event = append_size_dimensions(
            event,
            "visual_width",
            "visual_height",
            observation.visual_viewport.present.then_some(ViewportSize {
                width: observation.visual_viewport.width,
                height: observation.visual_viewport.height,
            }),
        );
    } else {
        event = event.field(DiagnosticField::boolean("dom_observation_present", false));
    }

    koushi_diagnostics::record(event);
}

fn append_size_dimensions(
    event: koushi_diagnostics::DiagnosticEvent,
    width_key: &'static str,
    height_key: &'static str,
    size: Option<ViewportSize>,
) -> koushi_diagnostics::DiagnosticEvent {
    use koushi_diagnostics::DiagnosticField;

    match size {
        Some(size) => event
            .field(DiagnosticField::count(
                width_key,
                normalized_dimension(size.width),
            ))
            .field(DiagnosticField::count(
                height_key,
                normalized_dimension(size.height),
            )),
        None => event
            .field(DiagnosticField::token(width_key, "none"))
            .field(DiagnosticField::token(height_key, "none")),
    }
}

fn append_rect_dimensions(
    event: koushi_diagnostics::DiagnosticEvent,
    width_key: &'static str,
    height_key: &'static str,
    rect: Option<ViewportRect>,
) -> koushi_diagnostics::DiagnosticEvent {
    append_size_dimensions(
        event,
        width_key,
        height_key,
        rect.map(|rect| ViewportSize {
            width: rect.width,
            height: rect.height,
        }),
    )
}

fn normalized_dimension(value: f64) -> u64 {
    value.round().clamp(0.0, MAX_VIEWPORT_DIMENSION) as u64
}

fn offset_class(value: f64) -> &'static str {
    if approx_zero(value) {
        "zero"
    } else if value.is_sign_negative() {
        "negative"
    } else {
        "positive"
    }
}

#[cfg(target_os = "macos")]
unsafe fn synchronize_macos_webview(
    platform_webview: tauri::webview::PlatformWebview,
    _trigger: ViewportSyncTrigger,
) -> NativeSyncResult {
    use objc2_app_kit::NSView;

    let webview: &NSView = unsafe { &*platform_webview.inner().cast() };
    let Some(parent) = (unsafe { webview.superview() }) else {
        return NativeSyncResult::unsupported();
    };

    let parent_bounds = parent.bounds();
    let current_frame = webview.frame();
    let parent_rect = viewport_rect_from_native(parent_bounds);
    let current_rect = viewport_rect_from_native(current_frame);
    let decision = viewport_sync_decision(parent_rect, current_rect);

    if decision == ViewportSyncDecision::RepairToParentBounds {
        webview.setFrame(parent_bounds);
    }

    let final_rect = viewport_rect_from_native(webview.frame());
    let final_origin_aligned = approx_equal(parent_rect.top, final_rect.top)
        && approx_equal(parent_rect.left, final_rect.left);
    let final_size_aligned = approx_equal(parent_rect.width, final_rect.width)
        && approx_equal(parent_rect.height, final_rect.height);
    NativeSyncResult {
        support: NativeViewportSupport::Supported,
        decision,
        native_aligned: final_origin_aligned && final_size_aligned,
        native_origin_aligned: final_origin_aligned,
        native_size_aligned: final_size_aligned,
        parent: Some(parent_rect),
        webview: Some(final_rect),
    }
}

#[cfg(target_os = "macos")]
fn viewport_rect_from_native(rect: objc2_foundation::NSRect) -> ViewportRect {
    ViewportRect {
        top: rect.origin.y,
        left: rect.origin.x,
        width: rect.size.width,
        height: rect.size.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(top: f64, left: f64, width: f64, height: f64) -> ViewportRect {
        ViewportRect {
            top,
            left,
            width,
            height,
        }
    }

    fn observation(trigger: ViewportSyncTrigger) -> ViewportSyncObservation {
        ViewportSyncObservation {
            trigger,
            density: ViewportDensity::Default,
            window: ViewportSize {
                width: 1200.0,
                height: 800.0,
            },
            document: ViewportSize {
                width: 1200.0,
                height: 800.0,
            },
            visual_viewport: VisualViewportObservation {
                present: true,
                width: 1200.0,
                height: 800.0,
                offset_left: 0.0,
                offset_top: 0.0,
            },
            body: rect(0.0, 0.0, 1200.0, 800.0),
            root: rect(0.0, 0.0, 1200.0, 800.0),
        }
    }

    #[test]
    fn policy_mismatch_requires_repair_and_subpoint_match_is_in_sync() {
        let parent = rect(0.0, 0.0, 1200.0, 800.0);
        assert_eq!(
            viewport_sync_decision(parent, rect(0.0, 0.0, 1199.0, 800.0)),
            ViewportSyncDecision::RepairToParentBounds
        );
        assert_eq!(
            viewport_sync_decision(parent, rect(0.25, -0.25, 1199.75, 800.25)),
            ViewportSyncDecision::InSync
        );
    }

    #[test]
    fn receipt_keeps_repair_decision_but_reports_final_native_alignment() {
        let observation = observation(ViewportSyncTrigger::Resized);
        let receipt = ViewportSyncReceipt::from_native(
            4,
            observation.trigger,
            NativeSyncResult {
                support: NativeViewportSupport::Supported,
                decision: ViewportSyncDecision::RepairToParentBounds,
                native_aligned: true,
                native_origin_aligned: true,
                native_size_aligned: true,
                parent: Some(rect(0.0, 0.0, 1200.0, 800.0)),
                webview: Some(rect(0.0, 0.0, 1200.0, 800.0)),
            },
        )
        .with_dom_observation(&observation);

        assert_eq!(receipt.decision, ViewportSyncDecision::RepairToParentBounds);
        assert!(receipt.native_aligned);
        assert!(receipt.native_origin_aligned);
        assert!(receipt.native_size_aligned);
        assert!(receipt.dom_js_aligned);
        assert!(receipt.dom_root_aligned);
    }

    #[test]
    fn admitted_triggers_are_explicit_and_moved_or_panel_only_do_not_repair() {
        for trigger in [
            ViewportSyncTrigger::PageLoad,
            ViewportSyncTrigger::Resized,
            ViewportSyncTrigger::ScaleFactorChanged,
            ViewportSyncTrigger::DensityCommit,
            ViewportSyncTrigger::BrowserResize,
        ] {
            assert!(trigger.is_admitted());
        }
        assert!(!ViewportSyncTrigger::Moved.is_admitted());
        assert!(!ViewportSyncTrigger::Panel.is_admitted());
        assert_eq!(
            viewport_sync_decision(rect(0.0, 0.0, 100.0, 100.0), rect(0.0, 0.0, 90.0, 100.0)),
            ViewportSyncDecision::RepairToParentBounds
        );
    }

    #[test]
    fn validation_rejects_non_finite_or_incomplete_numbers_before_sync() {
        let mut invalid = observation(ViewportSyncTrigger::DensityCommit);
        invalid.window.width = f64::NAN;
        assert_eq!(
            validate_observation(&invalid),
            Err(ViewportSyncValidationError::NonFiniteMeasurement)
        );

        let mut incomplete = observation(ViewportSyncTrigger::BrowserResize);
        incomplete.root.height = 0.0;
        assert_eq!(
            validate_observation(&incomplete),
            Err(ViewportSyncValidationError::InvalidMeasurement)
        );

        let moved = observation(ViewportSyncTrigger::Moved);
        assert_eq!(
            validate_observation(&moved),
            Err(ViewportSyncValidationError::TriggerNotAdmitted)
        );
    }

    #[test]
    fn generation_is_monotonic_and_diagnostic_receipt_is_closed() {
        let generation = ViewportSyncGeneration::default();
        assert_eq!(generation.next(), Ok(1));
        assert_eq!(generation.next(), Ok(2));

        let observation = observation(ViewportSyncTrigger::DensityCommit);
        validate_observation(&observation).expect("fixture observation is valid");
        let receipt = ViewportSyncReceipt::from_native(
            2,
            observation.trigger,
            NativeSyncResult {
                support: NativeViewportSupport::Supported,
                decision: ViewportSyncDecision::InSync,
                native_aligned: true,
                native_origin_aligned: true,
                native_size_aligned: true,
                parent: Some(rect(0.0, 0.0, 1200.0, 800.0)),
                webview: Some(rect(0.0, 0.0, 1200.0, 800.0)),
            },
        )
        .with_dom_observation(&observation);
        assert_eq!(receipt.generation, 2);
        assert!(receipt.native_aligned);
        assert!(receipt.dom_aligned);
        assert!(receipt.dom_js_aligned);
        assert!(receipt.dom_root_aligned);

        let _guard = koushi_diagnostics::test_support::lock();
        record_diagnostic(&receipt, Some(&observation));
        let after = koushi_diagnostics::test_support::detail_snapshot();
        let event = &after
            .records
            .iter()
            .rev()
            .find(|record| {
                record.event.source == "desktop.viewport_sync"
                    && record.event.stage == "observed"
                    && record.event.fields.iter().any(|field| {
                        field.key == "generation"
                            && field.value == koushi_diagnostics::DiagnosticValue::Correlation(2)
                    })
            })
            .expect("viewport diagnostic must be present")
            .event;
        let generation = event
            .fields
            .iter()
            .find(|field| field.key == "generation")
            .expect("generation diagnostic must be present");
        assert_eq!(
            generation.value,
            koushi_diagnostics::DiagnosticValue::Correlation(2)
        );
        assert!(event.fields.iter().all(|field| !matches!(
            field.value,
            koushi_diagnostics::DiagnosticValue::Token(value)
                if value.contains("://") || value.contains('/')
        )));
    }
}
