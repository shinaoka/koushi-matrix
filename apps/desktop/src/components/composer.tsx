import {
  type ChangeEvent,
  type FormEvent,
  type KeyboardEvent,
  type MouseEvent,
  memo,
  useEffect,
  useId,
  useRef,
  useState
} from "react";
import {
  AtSign,
  Bold,
  Clock3,
  Code2,
  Italic,
  Link2,
  List,
  Paperclip,
  Send,
  Smile,
  X
} from "lucide-react";
import { t } from "../i18n/messages";
import type {
  MentionIntent,
  ResolveComposerKeyAction
} from "../domain/types";
import {
  IS_MAC_PLATFORM,
  applyMacEmacsAction,
  composerKeyEventFromDom,
  insertNewlineAtSelection,
  macEmacsActionFromEvent,
  shouldLetNativeImeHandleComposerKeyEvent,
  shouldResolveComposerKeyEvent
} from "../domain/composerKeyEvents";
import { EmojiPicker } from "./EmojiPicker";
import {
  canApplyResolvedComposerAction,
  isComposerImeEnter,
  useComposerKeyIntentSnapshot,
  useCompositionOwnedTextarea
} from "../domain/compositionLifecycle";
import {
  ICON_SIZE,
  EMPTY_MENTION_INTENT,
  ignoreComposerKeyAction,
  activeMentionQuery,
  appendMentionTarget,
  mentionDraftToken,
  mentionTargetKey,
  mentionPillLabel,
  initials,
  defaultScheduleDateTimeValue,
  scheduledSendTimestampFromInput,
  type MentionCandidate,
  type ComposerModeProp
} from "../app/uiShared";
import { EntityAvatar } from "./Shell";

export const Composer = memo(function Composer({
  composerMode,
  hasStagedUploads = false,
  isSending,
  mentionCandidates = [],
  mentionIntent = EMPTY_MENTION_INTENT,
  resolveComposerKeyAction = ignoreComposerKeyAction,
  draftKey = "default",
  roomName,
  value,
  onCancelReply,
  onAttachFiles = async () => undefined,
  onMentionIntentChange = () => undefined,
  onScheduleSend = async () => undefined,
  onSend,
  onValueChange
}: {
  composerMode: ComposerModeProp;
  hasStagedUploads?: boolean;
  isSending: boolean;
  mentionCandidates?: MentionCandidate[];
  mentionIntent?: MentionIntent;
  resolveComposerKeyAction?: ResolveComposerKeyAction;
  draftKey?: string;
  roomName: string;
  value: string;
  onCancelReply: () => void;
  onAttachFiles?: (files: File[]) => void | Promise<void>;
  onMentionIntentChange?: (intent: MentionIntent) => void;
  onScheduleSend?: (sendAtMs: number, body: string) => void | Promise<void>;
  onSend: (body: string) => void | Promise<void>;
  onValueChange: (value: string) => void;
}) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const emojiButtonRef = useRef<HTMLButtonElement>(null);
  const macKillRingRef = useRef<string>("");
  const [scheduleOpen, setScheduleOpen] = useState(false);
  const [emojiPickerOpen, setEmojiPickerOpen] = useState(false);
  const [scheduleValue, setScheduleValue] = useState(() => defaultScheduleDateTimeValue());
  const [localValue, setLocalValue] = useState(value);
  const [activeMentionIndex, setActiveMentionIndex] = useState(0);
  const [dismissedMentionKey, setDismissedMentionKey] = useState<string | null>(null);
  const {
    textareaRef,
    lifecycle: imeComposition,
    onCompositionStart,
    onCompositionEnd
  } = useCompositionOwnedTextarea(value, draftKey);
  const captureKeyIntent = useComposerKeyIntentSnapshot(imeComposition);
  const autocompleteListboxId = useId();
  const activeMention = activeMentionQuery(localValue);
  const activeMentionKey =
    activeMention === null ? null : `${activeMention.start}:${activeMention.query.toLowerCase()}`;
  const activeMentionSuggestions =
    activeMention === null || activeMentionKey === dismissedMentionKey
      ? []
      : mentionCandidates
          .filter((candidate) => candidate.searchText.includes(activeMention.query.toLowerCase()))
          .slice(0, 8);
  const autocompleteOpen = activeMentionSuggestions.length > 0;
  const mentionSuggestionSections = mentionSections(activeMentionSuggestions);
  const activeMentionOption = autocompleteOpen
    ? activeMentionSuggestions[Math.min(activeMentionIndex, activeMentionSuggestions.length - 1)]
    : undefined;
  const activeMentionOptionId =
    autocompleteOpen && activeMentionOption
      ? `${autocompleteListboxId}-option-${Math.min(activeMentionIndex, activeMentionSuggestions.length - 1)}`
      : undefined;

  useEffect(() => {
    if (imeComposition.active()) {
      return;
    }
    setLocalValue(value);
  }, [draftKey, imeComposition, value]);

  useEffect(() => {
    setActiveMentionIndex(0);
  }, [activeMentionKey]);

  useEffect(() => {
    setActiveMentionIndex((current) =>
      activeMentionSuggestions.length === 0
        ? 0
        : Math.min(current, activeMentionSuggestions.length - 1)
    );
  }, [activeMentionSuggestions.length]);

  function updateLocalValue(nextValue: string) {
    setLocalValue(nextValue);
    onValueChange(nextValue);
  }

  function replaceTextRange(
    start: number,
    end: number,
    replacement: string,
    cursorOffset = replacement.length
  ) {
    const nextValue = `${localValue.slice(0, start)}${replacement}${localValue.slice(end)}`;
    const cursor = start + cursorOffset;
    updateLocalValue(nextValue);
    requestAnimationFrame(() => {
      textareaRef.current?.focus();
      textareaRef.current?.setSelectionRange(cursor, cursor);
    });
  }

  function closeAutocompleteForCurrentQuery() {
    if (activeMentionKey) {
      setDismissedMentionKey(activeMentionKey);
    }
  }

  function acceptActiveMention() {
    const candidate =
      activeMentionSuggestions[Math.min(activeMentionIndex, activeMentionSuggestions.length - 1)];
    if (candidate) {
      acceptMention(candidate);
    }
  }

  function selectionRange(): { start: number; end: number } {
    const textarea = textareaRef.current;
    return {
      start: textarea?.selectionStart ?? localValue.length,
      end: textarea?.selectionEnd ?? localValue.length
    };
  }

  function keepComposerFocus(event: MouseEvent<HTMLButtonElement>) {
    event.preventDefault();
  }

  function applyInlineMarkdown(prefix: string, suffix = prefix, placeholder = "") {
    const { start, end } = selectionRange();
    const selected = localValue.slice(start, end) || placeholder;
    replaceTextRange(
      start,
      end,
      `${prefix}${selected}${suffix}`,
      prefix.length + selected.length + suffix.length
    );
  }

  function applyLinkMarkdown() {
    const { start, end } = selectionRange();
    const selected = localValue.slice(start, end) || "link";
    const replacement = `[${selected}](https://)`;
    replaceTextRange(start, end, replacement, replacement.length - 1);
  }

  function applyListMarkdown() {
    const { start, end } = selectionRange();
    const selected = localValue.slice(start, end);
    if (!selected) {
      replaceTextRange(start, end, "- ", 2);
      return;
    }
    const replacement = selected
      .split("\n")
      .map((line) => (line.startsWith("- ") ? line : `- ${line}`))
      .join("\n");
    replaceTextRange(start, end, replacement);
  }

  function insertMentionTrigger() {
    const { start, end } = selectionRange();
    replaceTextRange(start, end, "@");
  }

  function insertEmoji(emoji: string) {
    const { start, end } = selectionRange();
    replaceTextRange(start, end, emoji);
  }

  function acceptMention(candidate: MentionCandidate) {
    if (!activeMention) {
      return;
    }
    const token = `${mentionDraftToken(candidate.target)} `;
    updateLocalValue(
      `${localValue.slice(0, activeMention.start)}${token}${localValue.slice(activeMention.end)}`
    );
    onMentionIntentChange(appendMentionTarget(mentionIntent, candidate.target));
    const cursor = activeMention.start + token.length;
    requestAnimationFrame(() => {
      textareaRef.current?.focus();
      textareaRef.current?.setSelectionRange(cursor, cursor);
    });
  }

  async function onAttachFileChange(event: ChangeEvent<HTMLInputElement>) {
    const files = Array.from(event.currentTarget.files ?? []);
    event.currentTarget.value = "";
    if (files.length === 0) {
      return;
    }
    try {
      await onAttachFiles(files);
    } catch {
      // Upload failure is reported through the Rust-owned operation/event path.
    }
  }

  async function attachDroppedOrPastedFiles(files: File[]) {
    if (files.length === 0) {
      return;
    }
    try {
      await onAttachFiles(files);
    } catch {
      // Upload failure is reported through the Rust-owned operation/event path.
    }
  }

  function openScheduleForm() {
    setScheduleValue(defaultScheduleDateTimeValue());
    setScheduleOpen(true);
  }

  async function submitSchedule(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const sendAtMs = scheduledSendTimestampFromInput(scheduleValue);
    if (sendAtMs === null || !localValue.trim() || hasStagedUploads || isSending) {
      return;
    }
    await onScheduleSend(sendAtMs, localValue);
    setScheduleOpen(false);
  }

  function onComposerKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (composerImeShouldHandleKeyEvent(event, imeComposition.active())) {
      return;
    }
    // macOS native Emacs text-editing bindings (Ctrl+F/B/P/N/K/Y).
    // Must not fire during IME composition.
    if (IS_MAC_PLATFORM && !event.nativeEvent.isComposing && !imeComposition.active()) {
      const emacsAction = macEmacsActionFromEvent(event);
      if (emacsAction !== null) {
        event.preventDefault();
        const ta = event.currentTarget;
        const effect = applyMacEmacsAction(
          emacsAction,
          localValue,
          ta.selectionStart,
          ta.selectionEnd,
          macKillRingRef.current
        );
        if (effect !== null) {
          if (effect.newKillRing !== undefined) {
            macKillRingRef.current = effect.newKillRing;
          }
          if (effect.newValue !== undefined) {
            updateLocalValue(effect.newValue);
          }
          const pos = effect.newSelectionPos;
          requestAnimationFrame(() => ta.setSelectionRange(pos, pos));
        }
        return;
      }
    }
    if (autocompleteOpen) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const direction = event.key === "ArrowDown" ? 1 : -1;
        setActiveMentionIndex((current) =>
          (current + direction + activeMentionSuggestions.length) % activeMentionSuggestions.length
        );
        return;
      }
      if (event.key === "Tab") {
        event.preventDefault();
        acceptActiveMention();
        return;
      }
    }
    if (!shouldResolveComposerKeyEvent(event)) {
      return;
    }

    const textarea = event.currentTarget;
    const intent = captureKeyIntent(textarea);
    if (intent === null) {
      event.preventDefault();
      return;
    }
    const keyEvent = composerKeyEventFromDom(event, {
      start: intent.selectionStart,
      end: intent.selectionEnd
    });
    const resolverOptions = {
      autocomplete_open: autocompleteOpen,
      send_enabled: !isSending && (intent.value.trim().length > 0 || hasStagedUploads)
    };
    if (shouldLetNativeImeHandleComposerKeyEvent(keyEvent)) {
      void resolveComposerKeyAction("main", keyEvent, resolverOptions)
        .catch(() => undefined)
        .finally(intent.releaseResolution);
      return;
    }
    event.preventDefault();

    void resolveComposerKeyAction("main", keyEvent, resolverOptions)
      .then((action) => {
        if (!canApplyResolvedComposerAction(intent, action)) {
          return;
        }
        if (action === "send") {
          void onSend(intent.value);
          return;
        }
        if (action === "insertNewline") {
          const nextValue = insertNewlineAtSelection(
            intent.value,
            intent.selectionStart,
            intent.selectionEnd
          );
          updateLocalValue(nextValue.value);
          requestAnimationFrame(() => {
            textarea.selectionStart = nextValue.cursor;
            textarea.selectionEnd = nextValue.cursor;
          });
          return;
        }
        if (action === "acceptAutocomplete") {
          acceptActiveMention();
          return;
        }
        if (action === "closeAutocomplete") {
          closeAutocompleteForCurrentQuery();
          return;
        }
        if (action === "cancel" && composerMode.kind === "reply") {
          onCancelReply();
        }
      })
      .catch(() => undefined)
      .finally(intent.releaseResolution);
  }

  return (
    <section className="composer" aria-label={t("composer.messageComposer")}>
      {composerMode.kind === "reply" ? (
        <div className="composer-reply-banner">
          <span className="composer-reply-label">{t("composer.replying")}</span>
          <button
            className="icon-button"
            type="button"
            aria-label={t("composer.cancelReply")}
            onClick={onCancelReply}
          >
            <X size={ICON_SIZE.small} />
          </button>
        </div>
      ) : null}
      <div className="composer-tools">
        <button
          className="icon-button"
          type="button"
          aria-label={t("composer.bold")}
          onMouseDown={keepComposerFocus}
          onClick={() => applyInlineMarkdown("**", "**", "bold")}
        >
          <Bold size={ICON_SIZE.input} />
        </button>
        <button
          className="icon-button"
          type="button"
          aria-label={t("composer.italic")}
          onMouseDown={keepComposerFocus}
          onClick={() => applyInlineMarkdown("_", "_", "italic")}
        >
          <Italic size={ICON_SIZE.input} />
        </button>
        <button
          className="icon-button"
          type="button"
          aria-label={t("composer.link")}
          onMouseDown={keepComposerFocus}
          onClick={applyLinkMarkdown}
        >
          <Link2 size={ICON_SIZE.input} />
        </button>
        <button
          className="icon-button"
          type="button"
          aria-label={t("composer.list")}
          onMouseDown={keepComposerFocus}
          onClick={applyListMarkdown}
        >
          <List size={ICON_SIZE.input} />
        </button>
        <button
          className="icon-button"
          type="button"
          aria-label={t("composer.code")}
          onMouseDown={keepComposerFocus}
          onClick={() => applyInlineMarkdown("`", "`", "code")}
        >
          <Code2 size={ICON_SIZE.input} />
        </button>
      </div>
      {mentionIntent.targets.length ? (
        <div className="composer-mention-pills" aria-label={t("composer.selectedMentions")}>
          {mentionIntent.targets.map((target) => (
            <span className="mention-pill" key={mentionTargetKey(target)} dir="auto">
              {mentionPillLabel(target)}
            </span>
          ))}
        </div>
      ) : null}
      {autocompleteOpen ? (
        <div
          id={autocompleteListboxId}
          className="composer-autocomplete"
          role="listbox"
          aria-label={t("composer.mentionSuggestions")}
          aria-activedescendant={activeMentionOptionId}
        >
          {mentionSuggestionSections.map((section) => (
            <div className="composer-autocomplete-section" key={section.key} role="presentation">
              <div className="composer-autocomplete-section-heading">{section.label}</div>
              {section.candidates.map(({ candidate, index }) => (
                <MentionOption
                  active={index === activeMentionIndex}
                  candidate={candidate}
                  id={`${autocompleteListboxId}-option-${index}`}
                  key={candidate.key}
                  onAccept={acceptMention}
                  onMouseDown={keepComposerFocus}
                />
              ))}
            </div>
          ))}
        </div>
      ) : null}
      <textarea
        ref={textareaRef}
        aria-label={t("composer.messageComposer")}
        defaultValue={localValue}
        placeholder={t("composer.placeholder", { roomName })}
        onKeyDown={onComposerKeyDown}
        onCompositionStart={onCompositionStart}
        onCompositionEnd={onCompositionEnd}
        onPaste={(event) => {
          const files = Array.from(event.clipboardData.files);
          if (files.length > 0) {
            event.preventDefault();
            void attachDroppedOrPastedFiles(files);
          }
        }}
        onChange={(event) => updateLocalValue(event.target.value)}
      />
      <div
        className="composer-footer"
        onDragOver={(event) => {
          event.preventDefault();
        }}
        onDrop={(event) => {
          const files = Array.from(event.dataTransfer.files);
          if (files.length > 0) {
            event.preventDefault();
            void attachDroppedOrPastedFiles(files);
          }
        }}
      >
        <div>
          <input
            ref={fileInputRef}
            className="composer-file-input"
            type="file"
            multiple
            aria-label={t("composer.attachFileInput")}
            onChange={(event) => {
              void onAttachFileChange(event);
            }}
          />
          <button
            className="icon-button"
            type="button"
            aria-label={t("composer.attachFile")}
            onClick={() => fileInputRef.current?.click()}
          >
            <Paperclip size={ICON_SIZE.control} />
          </button>
          <button
            className="icon-button"
            type="button"
            aria-label={t("composer.mention")}
            onMouseDown={keepComposerFocus}
            onClick={insertMentionTrigger}
          >
            <AtSign size={ICON_SIZE.control} />
          </button>
          <span className="composer-emoji-anchor">
            <button
              ref={emojiButtonRef}
              className="icon-button"
              type="button"
              aria-label={t("composer.emoji")}
              aria-expanded={emojiPickerOpen}
              aria-haspopup="dialog"
              onClick={() => setEmojiPickerOpen((open) => !open)}
            >
              <Smile size={ICON_SIZE.control} />
            </button>
            {emojiPickerOpen ? (
              <EmojiPicker
                anchorRef={emojiButtonRef}
                onSelect={insertEmoji}
                onClose={() => setEmojiPickerOpen(false)}
              />
            ) : null}
          </span>
          <button
            className="icon-button"
            type="button"
            aria-label={t("scheduled.sendLater")}
            disabled={isSending || !localValue.trim() || hasStagedUploads}
            onClick={openScheduleForm}
          >
            <Clock3 size={ICON_SIZE.control} />
          </button>
        </div>
        <button
          className={`send-button ${(localValue.trim() || hasStagedUploads) && !isSending ? "ready" : ""} ${isSending ? "is-sending" : ""}`}
          type="button"
          aria-label={isSending ? t("action.sending") : t("action.send")}
          disabled={isSending || (!localValue.trim() && !hasStagedUploads)}
          onClick={() => onSend(localValue)}
        >
          <Send size={ICON_SIZE.input} />
        </button>
      </div>
      {scheduleOpen ? (
        <form className="scheduled-send-form" onSubmit={submitSchedule}>
          <label className="scheduled-send-field">
            <span>{t("scheduled.timeInput")}</span>
            <input
              aria-label={t("scheduled.timeInput")}
              type="datetime-local"
              value={scheduleValue}
              onChange={(event) => setScheduleValue(event.currentTarget.value)}
            />
          </label>
          <div className="scheduled-send-form-actions">
            <button className="dialog-button" type="button" onClick={() => setScheduleOpen(false)}>
              {t("action.cancel")}
            </button>
            <button
              className="dialog-button is-primary"
              type="submit"
              disabled={scheduledSendTimestampFromInput(scheduleValue) === null}
            >
              {t("scheduled.schedule")}
            </button>
          </div>
        </form>
      ) : null}
    </section>
  );
});

type MentionSection = {
  key: "users" | "room";
  label: string;
  candidates: Array<{ candidate: MentionCandidate; index: number }>;
};

function mentionSections(candidates: MentionCandidate[]): MentionSection[] {
  const users: MentionSection["candidates"] = [];
  const roomMentions: MentionSection["candidates"] = [];
  candidates.forEach((candidate, index) => {
    const item = { candidate, index };
    if (candidate.target.kind === "roomMention") {
      roomMentions.push(item);
    } else {
      users.push(item);
    }
  });
  return [
    ...(users.length ? [{ key: "users" as const, label: t("composer.mentionUsers"), candidates: users }] : []),
    ...(roomMentions.length
      ? [
          {
            key: "room" as const,
            label: t("composer.mentionRoomNotification"),
            candidates: roomMentions
          }
        ]
      : [])
  ];
}

function MentionOption({
  active,
  candidate,
  id,
  onAccept,
  onMouseDown
}: {
  active: boolean;
  candidate: MentionCandidate;
  id: string;
  onAccept: (candidate: MentionCandidate) => void;
  onMouseDown: (event: MouseEvent<HTMLButtonElement>) => void;
}) {
  const meta = mentionOptionMeta(candidate);
  return (
    <button
      id={id}
      className={`composer-autocomplete-option ${active ? "is-active" : ""}`}
      key={candidate.key}
      type="button"
      role="option"
      aria-label={mentionOptionAriaLabel(candidate)}
      aria-selected={active ? "true" : "false"}
      onMouseDown={onMouseDown}
      onClick={() => onAccept(candidate)}
    >
      <EntityAvatar
        avatar={candidate.avatar ?? null}
        className={`mention-option-avatar ${
          candidate.target.kind === "roomMention" ? "is-room-mention" : "is-user"
        }`}
        colorSeed={mentionTargetKey(candidate.target)}
        fallback={candidate.target.kind === "roomMention" ? "@" : initials(candidate.label)}
      />
      <span className="mention-option-main">
        <span className="mention-option-label" dir="auto">
          {candidate.label}
        </span>
        <span className="mention-option-meta" dir="auto" aria-hidden="true">
          {meta}
        </span>
      </span>
    </button>
  );
}

function mentionOptionMeta(candidate: MentionCandidate): string {
  switch (candidate.target.kind) {
    case "user":
      return candidate.target.user_id;
    case "room":
      return candidate.target.room_id;
    case "roomMention":
      return t("composer.mentionRoomNotificationDescription");
  }
}

function mentionOptionAriaLabel(candidate: MentionCandidate): string {
  const meta = mentionOptionMeta(candidate);
  return meta ? `${candidate.label} ${meta}` : candidate.label;
}

function ThreadComposer({
  canEdit,
  draft,
  draftKey,
  isSending,
  resolveComposerKeyAction,
  onDraftChange,
  onSend
}: {
  canEdit: boolean;
  draft: string;
  draftKey: string;
  isSending: boolean;
  resolveComposerKeyAction: ResolveComposerKeyAction;
  onDraftChange: (draft: string) => void;
  onSend: (value: string) => void | Promise<void>;
}) {
  const [visibleDraft, setVisibleDraft] = useState(draft);
  const canSend = canEdit && !isSending && visibleDraft.trim().length > 0;
  const macKillRingRef = useRef<string>("");
  const {
    textareaRef,
    lifecycle: imeComposition,
    onCompositionStart,
    onCompositionEnd
  } = useCompositionOwnedTextarea(draft, draftKey);
  const captureKeyIntent = useComposerKeyIntentSnapshot(imeComposition);

  useEffect(() => {
    if (!imeComposition.active()) {
      setVisibleDraft(draft);
    }
  }, [draft, draftKey, imeComposition]);

  function updateVisibleDraft(value: string) {
    setVisibleDraft(value);
    onDraftChange(value);
  }

  function onComposerKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (composerImeShouldHandleKeyEvent(event, imeComposition.active())) {
      return;
    }
    // macOS native Emacs text-editing bindings (Ctrl+F/B/P/N/K/Y).
    // Must not fire during IME composition.
    if (IS_MAC_PLATFORM && !event.nativeEvent.isComposing && !imeComposition.active()) {
      const emacsAction = macEmacsActionFromEvent(event);
      if (emacsAction !== null) {
        event.preventDefault();
        const ta = event.currentTarget;
        const effect = applyMacEmacsAction(
          emacsAction,
          event.currentTarget.value,
          ta.selectionStart,
          ta.selectionEnd,
          macKillRingRef.current
        );
        if (effect !== null) {
          if (effect.newKillRing !== undefined) {
            macKillRingRef.current = effect.newKillRing;
          }
          if (effect.newValue !== undefined) {
            updateVisibleDraft(effect.newValue);
          }
          const pos = effect.newSelectionPos;
          requestAnimationFrame(() => ta.setSelectionRange(pos, pos));
        }
        return;
      }
    }
    if (!shouldResolveComposerKeyEvent(event)) {
      return;
    }

    const textarea = event.currentTarget;
    const intent = captureKeyIntent(textarea);
    if (intent === null) {
      event.preventDefault();
      return;
    }
    const keyEvent = composerKeyEventFromDom(event, {
      start: intent.selectionStart,
      end: intent.selectionEnd
    });
    const resolverOptions = {
      autocomplete_open: false,
      send_enabled: canEdit && !isSending && intent.value.trim().length > 0
    };
    if (shouldLetNativeImeHandleComposerKeyEvent(keyEvent)) {
      void resolveComposerKeyAction("thread", keyEvent, resolverOptions)
        .catch(() => undefined)
        .finally(intent.releaseResolution);
      return;
    }
    event.preventDefault();

    void resolveComposerKeyAction("thread", keyEvent, resolverOptions)
      .then((action) => {
        if (!canApplyResolvedComposerAction(intent, action)) {
          return;
        }
        if (action === "send") {
          void onSend(intent.value);
          return;
        }
        if (action === "insertNewline") {
          const nextDraft = insertNewlineAtSelection(
            intent.value,
            intent.selectionStart,
            intent.selectionEnd
          );
          updateVisibleDraft(nextDraft.value);
          requestAnimationFrame(() => {
            textarea.selectionStart = nextDraft.cursor;
            textarea.selectionEnd = nextDraft.cursor;
          });
        }
      })
      .catch(() => undefined)
      .finally(intent.releaseResolution);
  }

  return (
    <section className="thread-composer" aria-label={t("timeline.threadComposer")}>
      <textarea
        aria-label={t("timeline.threadComposer")}
        disabled={!canEdit}
        placeholder={t("timeline.threadPlaceholder")}
        ref={textareaRef}
        defaultValue={draft}
        onChange={(event) => updateVisibleDraft(event.target.value)}
        onKeyDown={onComposerKeyDown}
        onCompositionStart={onCompositionStart}
        onCompositionEnd={onCompositionEnd}
      />
      <div className="thread-composer-footer">
        <button
          className={`send-button ${canSend ? "ready" : ""} ${isSending ? "is-sending" : ""}`}
          type="button"
          aria-label={isSending ? t("action.sending") : t("action.send")}
          disabled={!canSend}
          onClick={() => {
            const value = textareaRef.current?.value ?? visibleDraft;
            void onSend(value);
          }}
        >
          <Send size={ICON_SIZE.input} />
        </button>
      </div>
    </section>
  );
}

function composerImeShouldHandleKeyEvent(
  event: KeyboardEvent<HTMLTextAreaElement>,
  compositionActive: boolean
): boolean {
  return isComposerImeEnter(event.key, {
    epochActive: compositionActive,
    nativeIsComposing: event.nativeEvent.isComposing,
    keyCode: event.keyCode
  });
}

export { ThreadComposer };
