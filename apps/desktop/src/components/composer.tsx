import {
  type ChangeEvent,
  type FormEvent,
  type KeyboardEvent,
  type MouseEvent,
  memo,
  useEffect,
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
  composerKeyEventFromDom,
  insertNewlineAtSelection,
  shouldLetNativeImeHandleComposerKeyEvent,
  shouldResolveComposerKeyEvent
} from "../domain/composerKeyEvents";
import { EmojiPicker } from "./EmojiPicker";
import {
  ICON_SIZE,
  EMPTY_MENTION_INTENT,
  ignoreComposerKeyAction,
  activeMentionQuery,
  appendMentionTarget,
  mentionDraftToken,
  mentionTargetKey,
  mentionPillLabel,
  defaultScheduleDateTimeValue,
  scheduledSendTimestampFromInput,
  type MentionCandidate,
  type ComposerModeProp
} from "../app/uiShared";

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
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const emojiButtonRef = useRef<HTMLButtonElement>(null);
  const imeCompositionActiveRef = useRef(false);
  const [scheduleOpen, setScheduleOpen] = useState(false);
  const [emojiPickerOpen, setEmojiPickerOpen] = useState(false);
  const [scheduleValue, setScheduleValue] = useState(() => defaultScheduleDateTimeValue());
  const [localValue, setLocalValue] = useState(value);
  const activeMention = activeMentionQuery(localValue);
  const activeMentionSuggestions =
    activeMention === null
      ? []
      : mentionCandidates
          .filter((candidate) => candidate.searchText.includes(activeMention.query.toLowerCase()))
          .slice(0, 5);
  const autocompleteOpen = activeMentionSuggestions.length > 0;

  useEffect(() => {
    setLocalValue(value);
  }, [draftKey, value]);

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
    if (composerImeShouldHandleKeyEvent(event, imeCompositionActiveRef.current)) {
      return;
    }
    if (!shouldResolveComposerKeyEvent(event)) {
      return;
    }

    const textarea = event.currentTarget;
    const selectionStart = textarea.selectionStart;
    const selectionEnd = textarea.selectionEnd;
    const keyEvent = composerKeyEventFromDom(event, {
      start: selectionStart,
      end: selectionEnd
    });
    const resolverOptions = {
      autocomplete_open: autocompleteOpen,
      send_enabled: !isSending && (localValue.trim().length > 0 || hasStagedUploads)
    };
    if (shouldLetNativeImeHandleComposerKeyEvent(keyEvent)) {
      void resolveComposerKeyAction("main", keyEvent, resolverOptions).catch(() => undefined);
      return;
    }
    event.preventDefault();

    void resolveComposerKeyAction("main", keyEvent, resolverOptions)
      .then((action) => {
        if (action === "send") {
          void onSend(localValue);
          return;
        }
        if (action === "insertNewline") {
          const nextValue = insertNewlineAtSelection(localValue, selectionStart, selectionEnd);
          updateLocalValue(nextValue.value);
          requestAnimationFrame(() => {
            textarea.selectionStart = nextValue.cursor;
            textarea.selectionEnd = nextValue.cursor;
          });
          return;
        }
        if (action === "acceptAutocomplete") {
          const firstSuggestion = activeMentionSuggestions[0];
          if (firstSuggestion) {
            acceptMention(firstSuggestion);
          }
          return;
        }
        if (action === "cancel" && composerMode.kind === "reply") {
          onCancelReply();
        }
      })
      .catch(() => undefined);
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
          className="composer-autocomplete"
          role="listbox"
          aria-label={t("composer.mentionSuggestions")}
        >
          {activeMentionSuggestions.map((candidate) => (
            <button
              className="composer-autocomplete-option"
              key={candidate.key}
              type="button"
              role="option"
              aria-label={candidate.label}
              aria-selected="false"
              onMouseDown={keepComposerFocus}
              onClick={() => acceptMention(candidate)}
            >
              <span className="mention-option-label" dir="auto">
                {candidate.label}
              </span>
              {candidate.target.kind === "user" ? (
                <span className="mention-option-meta" dir="auto" aria-hidden="true">
                  {candidate.target.user_id}
                </span>
              ) : null}
            </button>
          ))}
        </div>
      ) : null}
      <textarea
        ref={textareaRef}
        aria-label={t("composer.messageComposer")}
        value={localValue}
        placeholder={t("composer.placeholder", { roomName })}
        onKeyDown={onComposerKeyDown}
        onCompositionStart={() => {
          imeCompositionActiveRef.current = true;
        }}
        onCompositionEnd={() => {
          window.setTimeout(() => {
            imeCompositionActiveRef.current = false;
          }, 0);
        }}
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

function ThreadComposer({
  canEdit,
  draft,
  isSending,
  resolveComposerKeyAction,
  onDraftChange,
  onSend
}: {
  canEdit: boolean;
  draft: string;
  isSending: boolean;
  resolveComposerKeyAction: ResolveComposerKeyAction;
  onDraftChange: (draft: string) => void;
  onSend: () => void | Promise<void>;
}) {
  const canSend = canEdit && !isSending && draft.trim().length > 0;
  const imeCompositionActiveRef = useRef(false);

  function onComposerKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (composerImeShouldHandleKeyEvent(event, imeCompositionActiveRef.current)) {
      return;
    }
    if (!shouldResolveComposerKeyEvent(event)) {
      return;
    }

    const textarea = event.currentTarget;
    const selectionStart = textarea.selectionStart;
    const selectionEnd = textarea.selectionEnd;
    const keyEvent = composerKeyEventFromDom(event, {
      start: selectionStart,
      end: selectionEnd
    });
    const resolverOptions = {
      autocomplete_open: false,
      send_enabled: canSend
    };
    if (shouldLetNativeImeHandleComposerKeyEvent(keyEvent)) {
      void resolveComposerKeyAction("thread", keyEvent, resolverOptions).catch(() => undefined);
      return;
    }
    event.preventDefault();

    void resolveComposerKeyAction("thread", keyEvent, resolverOptions)
      .then((action) => {
        if (action === "send") {
          void onSend();
          return;
        }
        if (action === "insertNewline") {
          const nextDraft = insertNewlineAtSelection(draft, selectionStart, selectionEnd);
          onDraftChange(nextDraft.value);
          requestAnimationFrame(() => {
            textarea.selectionStart = nextDraft.cursor;
            textarea.selectionEnd = nextDraft.cursor;
          });
        }
      })
      .catch(() => undefined);
  }

  return (
    <section className="thread-composer" aria-label={t("timeline.threadComposer")}>
      <textarea
        aria-label={t("timeline.threadComposer")}
        disabled={!canEdit}
        placeholder={t("timeline.threadPlaceholder")}
        value={draft}
        onChange={(event) => onDraftChange(event.target.value)}
        onKeyDown={onComposerKeyDown}
        onCompositionStart={() => {
          imeCompositionActiveRef.current = true;
        }}
        onCompositionEnd={() => {
          window.setTimeout(() => {
            imeCompositionActiveRef.current = false;
          }, 0);
        }}
      />
      <div className="thread-composer-footer">
        <button
          className={`send-button ${canSend ? "ready" : ""} ${isSending ? "is-sending" : ""}`}
          type="button"
          aria-label={isSending ? t("action.sending") : t("action.send")}
          disabled={!canSend}
          onClick={onSend}
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
  return (
    event.key === "Enter" &&
    (compositionActive ||
      event.nativeEvent.isComposing ||
      event.keyCode === 229)
  );
}

export { ThreadComposer };
