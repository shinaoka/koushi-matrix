// Dialog components extracted from App.tsx.
// Imports: React, lucide-react, i18n, domain types, uiShared.

import {
  type KeyboardEvent,
  useState
} from "react";
import {
  Copy,
  FileText,
  Image as ImageIcon,
  X
} from "lucide-react";
import { t } from "../i18n/messages";
import type {
  CreateRoomVisibility,
  InviteScopeSelection,
  InviteWorkflowState,
  StagedUploadCompressionChoice,
  StagedUploadItem
} from "../domain/types";
import {
  ICON_SIZE,
  formatUploadBytes,
  formatUploadDimensions,
  captionBody,
  type ImageUploadVariantKindPayload,
  type ImageCompressionPlan
} from "../app/uiShared";

async function writeClipboardText(value: string): Promise<void> {
  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value);
  }
}

// ===== CreateEntityDialog =====

export interface CreateRoomDialogOptions {
  aliasLocalpart: string;
  encrypted: boolean;
  topic: string;
  visibility: CreateRoomVisibility;
}

export function CreateEntityDialog({
  activeSpaceName = null,
  isBusy,
  kind,
  roomOptions,
  value,
  onCancel,
  onRoomOptionsChange,
  onSubmit,
  onValueChange
}: {
  activeSpaceName?: string | null;
  isBusy: boolean;
  kind: "room" | "space";
  roomOptions?: CreateRoomDialogOptions;
  value: string;
  onCancel: () => void;
  onRoomOptionsChange?: (options: CreateRoomDialogOptions) => void;
  onSubmit: () => void;
  onValueChange: (value: string) => void;
}) {
  const isSpace = kind === "space";
  const effectiveRoomOptions =
    roomOptions ??
    ({
      aliasLocalpart: "",
      encrypted: true,
      topic: "",
      visibility: "private"
    } satisfies CreateRoomDialogOptions);
  const title = isSpace ? t("dialog.createSpaceTitle") : t("dialog.createRoomTitle");
  const inputLabel = isSpace ? t("dialog.spaceName") : t("dialog.roomName");
  const submitLabel = isSpace
    ? t("dialog.submitCreateSpace")
    : t("dialog.submitCreateRoom");
  const canSubmit =
    value.trim().length > 0 &&
    (isSpace ||
      effectiveRoomOptions.visibility === "private" ||
      effectiveRoomOptions.aliasLocalpart.trim().length > 0) &&
    !isBusy;

  function onDialogKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onCancel();
    }
  }

  function updateRoomOptions(patch: Partial<CreateRoomDialogOptions>) {
    const next = {
      ...effectiveRoomOptions,
      ...patch
    };
    if (next.visibility === "public") {
      next.encrypted = false;
    }
    onRoomOptionsChange?.(next);
  }

  return (
    <div
      className="dialog-overlay"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      onKeyDown={onDialogKeyDown}
    >
      <form
        className="dialog-box"
        onSubmit={(event) => {
          event.preventDefault();
          if (canSubmit) {
            onSubmit();
          }
        }}
      >
        <div className="dialog-title">{title}</div>
        <input
          className="dialog-input"
          type="text"
          autoFocus
          aria-label={inputLabel}
          placeholder={inputLabel}
          value={value}
          onChange={(event) => onValueChange(event.target.value)}
        />
        {!isSpace ? (
          <div className="create-room-options">
            <div className="create-room-visibility" role="radiogroup" aria-label={t("dialog.roomVisibility")}>
              <label className="create-room-option">
                <input
                  type="radio"
                  name="create-room-visibility"
                  checked={effectiveRoomOptions.visibility === "private"}
                  onChange={() =>
                    updateRoomOptions({
                      aliasLocalpart: "",
                      visibility: "private"
                    })
                  }
                />
                <span>{t("dialog.privateRoom")}</span>
              </label>
              <label className="create-room-option">
                <input
                  type="radio"
                  name="create-room-visibility"
                  checked={effectiveRoomOptions.visibility === "public"}
                  onChange={() =>
                    updateRoomOptions({
                      encrypted: false,
                      visibility: "public"
                    })
                  }
                />
                <span>{t("dialog.publicRoom")}</span>
              </label>
            </div>
            {activeSpaceName && effectiveRoomOptions.visibility === "private" ? (
              <div className="create-room-space-note">
                {t("dialog.standardRoomInSpace", { spaceName: activeSpaceName })}
              </div>
            ) : null}
            {effectiveRoomOptions.visibility === "private" ? (
              <label className="dialog-checkbox">
                <input
                  type="checkbox"
                  checked={effectiveRoomOptions.encrypted}
                  aria-label={t("dialog.encryptedRoom")}
                  onChange={(event) =>
                    updateRoomOptions({
                      encrypted: event.currentTarget.checked
                    })
                  }
                />
                <span>{t("dialog.encryptedRoom")}</span>
              </label>
            ) : null}
            <input
              className="dialog-input"
              type="text"
              aria-label={t("dialog.roomTopic")}
              placeholder={t("dialog.roomTopic")}
              value={effectiveRoomOptions.topic}
              onChange={(event) =>
                updateRoomOptions({
                  topic: event.target.value
                })
              }
            />
            {effectiveRoomOptions.visibility === "public" ? (
              <input
                className="dialog-input"
                type="text"
                aria-label={t("dialog.roomAddress")}
                placeholder={t("dialog.roomAddress")}
                value={effectiveRoomOptions.aliasLocalpart}
                onChange={(event) =>
                  updateRoomOptions({
                    aliasLocalpart: event.target.value
                  })
                }
              />
            ) : null}
          </div>
        ) : null}
        <div className="dialog-actions">
          <button
            className="dialog-button"
            type="button"
            aria-label={t("dialog.cancelCreate")}
            onClick={onCancel}
          >
            {t("action.cancel")}
          </button>
          <button
            className="dialog-button is-primary"
            type="submit"
            aria-label={submitLabel}
            disabled={!canSubmit}
          >
            {isSpace ? t("action.createSpace") : t("action.createRoom")}
          </button>
        </div>
      </form>
    </div>
  );
}

// ===== ImageCompressionDialog =====

export function ImageCompressionDialog({
  plan,
  onCancel,
  onChoose
}: {
  plan: ImageCompressionPlan;
  onCancel: () => void;
  onChoose: (choice: ImageUploadVariantKindPayload, saveDefault: boolean) => void;
}) {
  const [saveDefault, setSaveDefault] = useState(false);

  function onDialogKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onCancel();
    }
  }

  return (
    <div
      className="dialog-overlay"
      role="dialog"
      aria-modal="true"
      aria-label={t("composer.imageCompressionTitle")}
      onKeyDown={onDialogKeyDown}
    >
      <div className="dialog-box image-compression-dialog">
        <div className="dialog-title">{t("composer.imageCompressionTitle")}</div>
        <div className="image-compression-preview">
          <img src={plan.compressed.previewUrl} alt={t("composer.imageCompressionPreviewAlt")} />
        </div>
        <div className="image-compression-options">
          <button
            className="image-compression-option"
            type="button"
            onClick={() => onChoose("Original", saveDefault)}
          >
            <span>{t("composer.imageCompressionOriginal")}</span>
            <strong>
              {formatUploadBytes(plan.original.byteCount)} · {formatUploadDimensions(plan.original.dimensions)}
            </strong>
          </button>
          <button
            className="image-compression-option is-preferred"
            type="button"
            autoFocus
            onClick={() => onChoose("Compressed", saveDefault)}
          >
            <span>{t("composer.imageCompressionCompressed")}</span>
            <strong>
              {formatUploadBytes(plan.compressed.byteCount)} · {formatUploadDimensions(plan.compressed.dimensions)}
            </strong>
          </button>
        </div>
        <label className="dialog-checkbox">
          <input
            type="checkbox"
            checked={saveDefault}
            onChange={(event) => setSaveDefault(event.currentTarget.checked)}
          />
          <span>{t("composer.imageCompressionSaveDefault")}</span>
        </label>
        <div className="dialog-actions">
          <button className="dialog-button" type="button" onClick={onCancel}>
            {t("dialog.cancel")}
          </button>
        </div>
      </div>
    </div>
  );
}

// ===== DiagnosticDialog =====

export function DiagnosticDialog({
  report,
  onClose
}: {
  report: string;
  onClose: () => void;
}) {
  function onDialogKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    }
  }

  return (
    <div
      className="dialog-overlay"
      role="dialog"
      aria-modal="true"
      aria-label={t("diagnostics.title")}
      onKeyDown={onDialogKeyDown}
    >
      <div className="dialog-box diagnostics-dialog">
        <div className="dialog-title-row">
          <div className="dialog-title">{t("diagnostics.title")}</div>
          <button
            className="icon-button"
            type="button"
            aria-label={t("action.close")}
            onClick={onClose}
          >
            <X size={ICON_SIZE.small} />
          </button>
        </div>
        <pre className="diagnostics-output">{report}</pre>
        <div className="dialog-actions">
          <button
            className="dialog-button is-primary"
            type="button"
            aria-label={t("diagnostics.copy")}
            onClick={() => {
              void writeClipboardText(report);
            }}
          >
            <Copy size={ICON_SIZE.small} />
            <span>{t("diagnostics.copy")}</span>
          </button>
        </div>
      </div>
    </div>
  );
}

// ===== UserIdDialog =====

export function UserIdDialog({
  inputLabel,
  isBusy,
  submitLabel,
  title,
  value,
  onCancel,
  onSubmit,
  onValueChange
}: {
  inputLabel: string;
  isBusy: boolean;
  submitLabel: string;
  title: string;
  value: string;
  onCancel: () => void;
  onSubmit: () => void;
  onValueChange: (value: string) => void;
}) {
  const canSubmit = value.trim().length > 0 && !isBusy;

  function onDialogKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onCancel();
    }
  }

  return (
    <div
      className="dialog-overlay"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      onKeyDown={onDialogKeyDown}
    >
      <form
        className="dialog-box"
        onSubmit={(event) => {
          event.preventDefault();
          if (canSubmit) {
            onSubmit();
          }
        }}
      >
        <div className="dialog-title">{title}</div>
        <input
          className="dialog-input"
          type="text"
          autoFocus
          aria-label={inputLabel}
          placeholder={inputLabel}
          spellCheck={false}
          value={value}
          onChange={(event) => onValueChange(event.target.value)}
        />
        <div className="dialog-actions">
          <button
            className="dialog-button"
            type="button"
            aria-label={t("action.cancel")}
            onClick={onCancel}
          >
            {t("action.cancel")}
          </button>
          <button
            className="dialog-button is-primary"
            type="submit"
            aria-label={submitLabel}
            disabled={!canSubmit}
          >
            {submitLabel}
          </button>
        </div>
      </form>
    </div>
  );
}

// ===== InviteTargetsDialog =====

export function InviteTargetsDialog({
  isBusy,
  query,
  scope,
  title,
  workflow,
  onCancel,
  onQueryChange,
  onRemoveTarget,
  onScopeChange,
  onSelectCandidate,
  onSubmit
}: {
  isBusy: boolean;
  query: string;
  scope: InviteScopeSelection;
  title: string;
  workflow: InviteWorkflowState;
  onCancel: () => void;
  onQueryChange: (value: string) => void;
  onRemoveTarget: (userId: string) => void;
  onScopeChange: (scope: InviteScopeSelection) => void;
  onSelectCandidate: (userId: string) => void;
  onSubmit: () => void;
}) {
  const isPending = workflow.operation.kind === "pending";
  const canSubmit = workflow.selected_targets.length > 0 && !isBusy && !isPending;

  function onDialogKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onCancel();
    }
  }

  return (
    <div
      className="dialog-overlay"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      onKeyDown={onDialogKeyDown}
    >
      <form
        className="dialog-box invite-target-dialog"
        onSubmit={(event) => {
          event.preventDefault();
          if (canSubmit) {
            onSubmit();
          }
        }}
      >
        <div className="dialog-title">{title}</div>
        {workflow.operation.kind === "completed" && workflow.operation.notice ? (
          <div className="invite-target-notice" role="status">
            {workflow.operation.notice}
          </div>
        ) : null}
        <div className="invite-selected-targets" aria-label={t("dialog.inviteSelectedTargets")}>
          {workflow.selected_targets.map((target) => (
            <span className="invite-selected-target" key={target.user_id}>
              <span>{target.display_label}</span>
              <button
                type="button"
                aria-label={t("dialog.removeInviteTarget")}
                onClick={() => onRemoveTarget(target.user_id)}
              >
                <X size={14} aria-hidden="true" />
              </button>
            </span>
          ))}
        </div>
        <input
          className="dialog-input"
          type="text"
          autoFocus
          aria-label={t("dialog.inviteSearch")}
          placeholder={t("dialog.inviteSearch")}
          spellCheck={false}
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
        />
        <div className="invite-target-candidates" aria-label={t("dialog.inviteCandidates")}>
          {workflow.query.candidates.map((candidate) => (
            <button
              type="button"
              key={candidate.user_id}
              className="invite-target-candidate"
              disabled={candidate.status !== "selectable"}
              onClick={() => onSelectCandidate(candidate.user_id)}
            >
              <span>{candidate.display_label}</span>
              <span>{candidate.user_id}</span>
            </button>
          ))}
          {workflow.query.explicit_user_id ? (
            <button
              type="button"
              className="invite-target-candidate"
              disabled={workflow.query.explicit_user_id.status !== "selectable"}
              onClick={() => onSelectCandidate(workflow.query.explicit_user_id!.user_id)}
            >
              <span>{workflow.query.explicit_user_id.display_label}</span>
              <span>
                {workflow.query.explicit_user_id.status === "invalidMatrixId"
                  ? t("dialog.inviteInvalidMatrixId")
                  : workflow.query.explicit_user_id.user_id}
              </span>
            </button>
          ) : null}
        </div>
        {workflow.scope_plan ? (
          <div className="invite-scope-options" aria-label={t("dialog.inviteScope")}>
            {workflow.scope_plan.options.map((option) => {
              const checked = inviteScopeKey(option.scope) === inviteScopeKey(scope);
              return (
                <label className="invite-scope-option" key={inviteScopeKey(option.scope)}>
                  <input
                    type="radio"
                    name="invite-scope"
                    checked={checked}
                    onChange={() => onScopeChange(option.scope)}
                  />
                  <span>{option.label}</span>
                </label>
              );
            })}
          </div>
        ) : null}
        <div className="dialog-actions">
          <button className="dialog-button" type="button" aria-label={t("action.cancel")} onClick={onCancel}>
            {t("action.cancel")}
          </button>
          <button className="dialog-button is-primary" type="submit" disabled={!canSubmit}>
            {t("dialog.sendInvite")}
          </button>
        </div>
      </form>
    </div>
  );
}

function inviteScopeKey(scope: InviteScopeSelection): string {
  return scope.kind === "roomOnly" ? "roomOnly" : `parent:${scope.space_id}`;
}

// ===== ReportReasonDialog =====

export function ReportReasonDialog({
  reason,
  title,
  onCancel,
  onReasonChange,
  onSubmit
}: {
  reason: string;
  title: string;
  onCancel: () => void;
  onReasonChange: (reason: string) => void;
  onSubmit: () => void;
}) {
  const canSubmit = reason.trim().length > 0;

  function onDialogKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onCancel();
    }
  }

  return (
    <div
      className="dialog-overlay"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      onKeyDown={onDialogKeyDown}
    >
      <form
        className="dialog-box"
        onSubmit={(event) => {
          event.preventDefault();
          if (canSubmit) {
            onSubmit();
          }
        }}
      >
        <div className="dialog-title">{title}</div>
        <label className="dialog-input-label">
          <span>{t("dialog.reportReasonLabel")}</span>
          <input
            className="dialog-input"
            type="text"
            autoFocus
            aria-label={t("dialog.reportReasonLabel")}
            placeholder={t("dialog.reportReasonPlaceholder")}
            value={reason}
            onChange={(event) => onReasonChange(event.target.value)}
          />
        </label>
        <div className="dialog-actions">
          <button
            className="dialog-button"
            type="button"
            aria-label={t("action.cancel")}
            onClick={onCancel}
          >
            {t("action.cancel")}
          </button>
          <button
            className="dialog-button is-primary"
            type="submit"
            aria-label={t("action.report")}
            disabled={!canSubmit}
          >
            {t("action.report")}
          </button>
        </div>
      </form>
    </div>
  );
}

// ===== UploadStagingDialog =====

export function UploadStagingDialog({
  items,
  onClear,
  onUpdateCaption,
  onUpdateCompression
}: {
  items: StagedUploadItem[];
  onClear: () => void | Promise<void>;
  onUpdateCaption: (stagedId: string, caption: string) => void | Promise<void>;
  onUpdateCompression: (
    stagedId: string,
    compressionChoice: StagedUploadCompressionChoice
  ) => void | Promise<void>;
}) {
  return (
    <section
      className="upload-staging-dialog"
      role="dialog"
      aria-label={t("upload.dialogTitle")}
    >
      <div className="upload-staging-header">
        <h2>{t("upload.dialogTitle")}</h2>
        <button className="icon-button" type="button" aria-label={t("upload.clear")} onClick={onClear}>
          <X size={ICON_SIZE.small} />
        </button>
      </div>
      <div className="upload-staging-list">
        {items.map((item) => (
          <article className="upload-staging-item" key={item.staged_id}>
            <div className="upload-staging-file">
              {item.kind.kind === "image" ? (
                <ImageIcon size={ICON_SIZE.control} aria-hidden="true" />
              ) : (
                <FileText size={ICON_SIZE.control} aria-hidden="true" />
              )}
              <span className="upload-staging-name" dir="auto">
                {item.filename || t("composer.attachmentFallback")}
              </span>
              <span className="upload-staging-meta">
                {formatUploadBytes(item.byte_count)}
              </span>
            </div>
            <label className="upload-staging-caption">
              <span>{t("upload.captionForFile", { filename: item.filename })}</span>
              <input
                value={captionBody(item)}
                aria-label={t("upload.captionForFile", { filename: item.filename })}
                onChange={(event) => {
                  void onUpdateCaption(item.staged_id, event.currentTarget.value);
                }}
              />
            </label>
            {item.kind.kind === "image" ? (
              <div className="upload-staging-choice" role="group" aria-label={t("upload.sizeChoice")}>
                <button
                  className="dialog-button"
                  type="button"
                  aria-pressed={item.compression_choice.kind === "original"}
                  onClick={() => {
                    void onUpdateCompression(item.staged_id, { kind: "original" });
                  }}
                >
                  {t("upload.original")}
                </button>
                <button
                  className="dialog-button"
                  type="button"
                  aria-pressed={item.compression_choice.kind === "ask"}
                  onClick={() => {
                    void onUpdateCompression(item.staged_id, { kind: "ask" });
                  }}
                >
                  {t("upload.ask")}
                </button>
                <button
                  className="dialog-button"
                  type="button"
                  aria-pressed={item.compression_choice.kind === "compressed"}
                  onClick={() => {
                    void onUpdateCompression(item.staged_id, {
                      kind: "compressed",
                      mode: "always"
                    });
                  }}
                >
                  {t("upload.compressed")}
                </button>
              </div>
            ) : null}
          </article>
        ))}
      </div>
    </section>
  );
}
