import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import { getActiveLocale, t } from "../../i18n/messages";
import { peopleFacingLabel } from "../../app/uiShared";
import {
  FloatingLayer,
  floatingPlacementStyle,
  useFloatingPlacement
} from "../floatingLayer";
import { timelineKeyIdentity, type ReceiptSourceRef, type ReaderRow } from "../../domain/coreEvents";
import { EntityAvatar } from "../Shell";
import { api } from "../../backend/appRuntime";
import type { LiveReadReceipt } from "../../domain/types";

/** Reader popup width; the panel narrows to the pane when it is smaller. */
const RECEIPT_POPUP_INLINE_SIZE_PX = 420;
/**
 * Reader popup height follows the row count (#360).
 *
 * A fixed height made the popup the same size for two readers as for six, and
 * because `.receipt-tooltip` is a grid, its auto rows stretched to fill the
 * slack — two readers rendered as two ~55px rows with a large blank gap. These
 * mirror the `--receipt-tooltip-*` CSS tokens; keep them in step.
 */
const RECEIPT_POPUP_ROW_BLOCK_SIZE_PX = 20;
const RECEIPT_POPUP_ROW_GAP_PX = 3;
const RECEIPT_POPUP_PADDING_BLOCK_PX = 8;
const RECEIPT_POPUP_BORDER_BLOCK_PX = 1;

function receiptPopupBlockSize(rowCount: number): number {
  const rows = Math.max(rowCount, 1);
  return (
    rows * RECEIPT_POPUP_ROW_BLOCK_SIZE_PX +
    (rows - 1) * RECEIPT_POPUP_ROW_GAP_PX +
    2 * (RECEIPT_POPUP_PADDING_BLOCK_PX + RECEIPT_POPUP_BORDER_BLOCK_PX)
  );
}

function containsTarget(container: Element | null, target: EventTarget | null): boolean {
  return typeof Node !== "undefined" && target instanceof Node && container?.contains(target) === true;
}

/**
 * Read-receipt avatar stack plus its reader popup.
 *
 * The popup renders in the body-level floating layer: the thread pane is
 * overflow-clipped, so a row-local popup gets cut off at the pane edge. Hover
 * and focus open the same popup so keyboard users reach what pointer users see.
 */
export function ReceiptReaders({
  overflowCount,
  receipts,
  source,
  totalCount,
  onRequestAvatarThumbnail
}: {
  overflowCount: number;
  receipts: LiveReadReceipt[];
  source?: ReceiptSourceRef;
  totalCount: number;
  onRequestAvatarThumbnail?: (mxcUri: string) => void | Promise<void | (() => void)>;
}) {
  const anchorRef = useRef<HTMLDivElement>(null);
  const popupRef = useRef<HTMLSpanElement>(null);
  const [open, setOpen] = useState(false);
  const [readerScope, setReaderScope] = useState<string | null>(null);
  const [readerRevision, setReaderRevision] = useState<string | null>(null);
  const [readerInstalledRevision, setReaderInstalledRevision] = useState<string | null>(null);
  const [readerResourceUrls, setReaderResourceUrls] = useState<Record<string, string>>({});
  const [readerStart, setReaderStart] = useState(0);
  const [readerTotal, setReaderTotal] = useState(0);
  const [readerWindowSequence, setReaderWindowSequence] = useState("0");
  const [focusedReaderUserId, setFocusedReaderUserId] = useState<string | null>(null);
  const [readerState, setReaderState] = useState<"loading" | "ready" | "failed">("loading");
  const compactRows = useMemo(() => receipts.map(compactReaderRow), [receipts]);
  const compactDetails = compactRows.map(formatReaderRow);
  if (overflowCount > 0) {
    compactDetails.push(t("timeline.readReceiptOverflow", { count: overflowCount }));
  }
  const receiptLabel = t("timeline.readBy", { count: totalCount });
  const receiptAriaLabel =
    compactDetails.length > 0 ? `${receiptLabel}: ${compactDetails.join("; ")}` : receiptLabel;
  const receiptTitle = compactDetails.join("\n");
  const [readerRows, setReaderRows] = useState<ReaderRow[]>(compactRows);
  const readerResourceRefs = useMemo(() => {
    const refs = new Set<string>();
    for (const row of readerRows) {
      if (row.avatar?.kind === "ready") {
        refs.add(row.avatar.source_ref);
      }
    }
    return [...refs];
  }, [readerRows]);
  const readerResourceIdentity = readerResourceRefs.join("\u0000");
  const appliedReaderRevisionRef = useRef<string | null>(null);
  const readerSequenceRef = useRef(0);
  const avatarObservationSequenceRef = useRef(0n);
  const readerRowRefs = useRef(new Map<string, HTMLSpanElement>());
  const pendingReaderFocusRef = useRef<{ index: number; sequence: string } | null>(null);
  const sourceRef = useRef(source);
  sourceRef.current = source;
  const sourceIdentity = source
    ? JSON.stringify([
        timelineKeyIdentity(source.key),
        source.projection_request_id.connection_id,
        source.projection_request_id.sequence,
        source.generation,
        source.event_id
      ])
    : null;
  const requestReaderWindowAt = useCallback(
    (start: number): string | null => {
      if (!readerScope || !readerRevision || readerTotal <= 0) return null;
      const nextStart = Math.max(0, Math.min(Math.trunc(start), readerTotal - 1));
      const sequence = String(++readerSequenceRef.current);
      setReaderWindowSequence(sequence);
      void api.updateReceiptReaderWindow(readerScope, {
        installed_revision: readerRevision,
        sequence,
        target: { kind: "index", start: String(nextStart) },
        limit: 256
      });
      return sequence;
    },
    [readerRevision, readerScope, readerTotal]
  );
  const focusReaderAt = useCallback(
    (index: number) => {
      const offset = index - readerStart;
      const row = readerRows[offset];
      if (row) {
        setFocusedReaderUserId(row.user_id);
        readerRowRefs.current.get(row.user_id)?.focus();
        return;
      }
      const requestStart = index >= readerTotal - 1 ? Math.max(readerTotal - 256, 0) : index;
      const sequence = requestReaderWindowAt(requestStart);
      if (sequence) pendingReaderFocusRef.current = { index, sequence };
    },
    [readerRows, readerStart, readerTotal, requestReaderWindowAt]
  );
  useLayoutEffect(() => {
    const pending = pendingReaderFocusRef.current;
    if (
      !pending ||
      pending.sequence !== readerWindowSequence ||
      pending.index < readerStart ||
      pending.index >= readerStart + readerRows.length
    ) {
      return;
    }
    const row = readerRows[pending.index - readerStart];
    if (!row) return;
    pendingReaderFocusRef.current = null;
    setFocusedReaderUserId(row.user_id);
    readerRowRefs.current.get(row.user_id)?.focus();
  }, [readerRows, readerStart, readerWindowSequence]);
  useEffect(() => {
    if (!open) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [open]);
  useEffect(() => {
    if (!open || sourceIdentity) return;
    // Until a committed source exists, keep the bounded summary usable.
    setReaderRows(compactRows);
    setReaderStart(0);
    setReaderTotal(totalCount);
    setReaderState("ready");
  }, [compactRows, open, sourceIdentity, totalCount]);
  useEffect(() => {
    const activeSource = sourceRef.current;
    if (!open || !activeSource) return;
    let cancelled = false;
    let scope: string | null = null;
    void api
      .subscribeReceiptReader(activeSource, 0, 256)
      .then((nextScope) => {
        if (cancelled) {
          void api.closeReceiptReader(nextScope);
        } else {
          scope = nextScope;
          setReaderScope(nextScope);
        }
      })
      .catch(() => {
        if (!cancelled) setReaderState("failed");
      });
    return () => {
      cancelled = true;
      if (scope) void api.closeReceiptReader(scope);
      setReaderScope(null);
      appliedReaderRevisionRef.current = null;
      readerSequenceRef.current = 0;
      avatarObservationSequenceRef.current = 0n;
      pendingReaderFocusRef.current = null;
      setFocusedReaderUserId(null);
      setReaderRevision(null);
      setReaderInstalledRevision(null);
      setReaderResourceUrls({});
      setReaderStart(0);
      setReaderTotal(0);
      setReaderRows([]);
      setReaderState("loading");
    };
  }, [open, sourceIdentity]);
  useEffect(() => {
    if (!readerScope) return;
    let cancelled = false;
    const receive = async (): Promise<void> => {
      while (!cancelled) {
        const delivery = await api.receiveReceiptReader(readerScope);
        if (cancelled || !delivery) return;
        if (delivery.kind === "retired") {
          setReaderRows([]);
          setReaderState("failed");
          return;
        }
        if (delivery.model.kind === "readerLoading") {
          setReaderState("loading");
          continue;
        }
        if (delivery.model.kind !== "readerReady") continue;
        if (appliedReaderRevisionRef.current === delivery.revision) {
          setReaderState("ready");
          continue;
        }
        appliedReaderRevisionRef.current = delivery.revision;
        setReaderState("ready");
        setReaderRevision(delivery.revision);
        setReaderStart(delivery.model.start);
        setReaderTotal(delivery.model.total_count);
        setReaderWindowSequence(delivery.model.window_sequence);
        setReaderRows(delivery.model.rows);
        await api.ackReceiptReader(readerScope, delivery.revision);
        if (!cancelled) setReaderInstalledRevision(delivery.revision);
      }
    };
    void receive().catch(() => {
      if (!cancelled) {
        setReaderRows([]);
        setReaderState("failed");
      }
    });
    return () => {
      cancelled = true;
    };
  }, [readerScope]);
  useEffect(() => {
    if (!readerScope || !readerInstalledRevision) {
      setReaderResourceUrls({});
      return;
    }
    const sourceRefs = readerResourceIdentity ? readerResourceIdentity.split("\u0000") : [];
    if (sourceRefs.length === 0) {
      setReaderResourceUrls({});
      return;
    }

    let cancelled = false;
    const ownedObjectUrls: string[] = [];
    const loadResources = async (): Promise<void> => {
      const entries: Array<readonly [string, string]> = [];
      // Keep host reads serialized. The window is bounded, but a Promise.all over
      // 256 rows would still create an avoidable burst of IPC responses and
      // renderer-side blob allocations.
      for (const sourceRef of sourceRefs) {
        if (cancelled) return;
        try {
          const content = await api.readReceiptReaderResource(
            readerScope,
            readerInstalledRevision,
            sourceRef
          );
          if (!content) continue;
          const resource = createReaderResourceUrl(content.bytes, content.mime_type);
          if (!resource || cancelled) {
            resource?.revoke?.();
            continue;
          }
          if (resource.revoke) ownedObjectUrls.push(resource.url);
          entries.push([sourceRef, resource.url]);
        } catch {
          // A missing or retired resource keeps the Rust-provided initials.
        }
      }
      if (cancelled) {
        for (const url of ownedObjectUrls) URL.revokeObjectURL(url);
        return;
      }
      setReaderResourceUrls(Object.fromEntries(entries));
    };
    void loadResources();
    return () => {
      cancelled = true;
      for (const url of ownedObjectUrls) URL.revokeObjectURL(url);
    };
  }, [readerInstalledRevision, readerResourceIdentity, readerScope]);
  useEffect(() => {
    const popup = popupRef.current;
    if (!open || !popup || !readerScope || !readerInstalledRevision || readerInstalledRevision !== readerRevision) return;
    let frame: number | null = null;
    let lastGeometry = "";
    const report = () => {
      frame = null;
      const bounds = popup.getBoundingClientRect();
      const visible: string[] = [];
      const nearby: Array<{ id: string; distance: number }> = [];
      if (bounds.height > 0 && bounds.width > 0) {
        for (const row of readerRows) {
          const rect = readerRowRefs.current.get(row.user_id)?.getBoundingClientRect();
          if (!rect || rect.height <= 0 || rect.width <= 0) continue;
          if (rect.bottom > bounds.top && rect.top < bounds.bottom) visible.push(row.user_id);
          else nearby.push({ id: row.user_id, distance: Math.max(bounds.top - rect.bottom, rect.top - bounds.bottom) });
        }
      }
      const prefetch = nearby.sort((a, b) => a.distance - b.distance).slice(0, 8).map((row) => row.id);
      const geometry = JSON.stringify([visible, prefetch]);
      if (geometry === lastGeometry) return;
      lastGeometry = geometry;
      void api.observeReceiptReaderAvatars(readerScope, {
        installed_revision: readerInstalledRevision,
        sequence: String(++avatarObservationSequenceRef.current),
        visible_user_ids: visible,
        prefetch_user_ids: prefetch
      }).catch(() => {
        // A newer Rust model/session may invalidate an in-flight observation.
        // Do not repair or retry Matrix demand in the renderer.
      });
    };
    const schedule = () => {
      if (frame === null) frame = requestAnimationFrame(report);
    };
    report();
    popup.addEventListener("scroll", schedule);
    window.addEventListener("resize", schedule);
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(schedule);
    resize?.observe(popup);
    for (const row of readerRows) {
      const element = readerRowRefs.current.get(row.user_id);
      if (element) resize?.observe(element);
    }
    return () => {
      if (frame !== null) cancelAnimationFrame(frame);
      popup.removeEventListener("scroll", schedule);
      window.removeEventListener("resize", schedule);
      resize?.disconnect();
    };
  }, [open, readerInstalledRevision, readerRevision, readerRows, readerScope]);
  const readerRemainingCount = source
    ? Math.max(readerTotal - readerStart - readerRows.length, 0)
    : Math.max(overflowCount, 0);
  const placement = useFloatingPlacement({
    align: "end",
    anchorRef,
    blockSize: receiptPopupBlockSize(readerRows.length + (readerRemainingCount > 0 ? 1 : 0)),
    inlineSize: RECEIPT_POPUP_INLINE_SIZE_PX,
    placement: "above",
    resolveBoundaryElement: receiptPopupBoundaryElement
  });

  return (
    <div
      ref={anchorRef}
      className="message-receipts"
      aria-label={receiptAriaLabel}
      aria-haspopup="dialog"
      role="button"
      tabIndex={0}
      title={receiptTitle}
      onBlur={(event) => {
        const nextTarget = event.relatedTarget;
        if (
          !nextTarget ||
          (!containsTarget(anchorRef.current, nextTarget) &&
            !containsTarget(popupRef.current, nextTarget))
        ) {
          setOpen(false);
        }
      }}
      onFocus={() => setOpen(true)}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          setOpen(true);
        }
      }}
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={(event) => {
        if (!containsTarget(popupRef.current, event.relatedTarget)) setOpen(false);
      }}
    >
      <span className="receipt-avatars" aria-hidden="true">
        {receipts.map((receipt) => (
          <EntityAvatar
            avatar={receipt.avatar}
            className="receipt-reader-avatar"
            colorSeed={receipt.user_id}
            fallback={receiptInitials(receipt)}
            key={receipt.user_id}
            onRequestAvatarThumbnail={onRequestAvatarThumbnail}
          />
        ))}
        {overflowCount > 0 ? <span className="receipt-overflow">+{overflowCount}</span> : null}
      </span>
      {open && (readerRows.length > 0 || readerState !== "ready") ? (
        <FloatingLayer>
          <span
            ref={popupRef}
            className="receipt-tooltip"
            role="dialog"
            aria-label={receiptLabel}
            aria-busy={readerState === "loading"}
            style={{
              ...floatingPlacementStyle(placement),
              maxBlockSize: "min(60vh, 420px)",
              overflowY: "auto"
            }}
            onBlur={(event) => {
              const nextTarget = event.relatedTarget;
              if (
                !nextTarget ||
                (!containsTarget(anchorRef.current, nextTarget) &&
                  !containsTarget(popupRef.current, nextTarget))
              ) {
                setOpen(false);
              }
            }}
            onMouseEnter={() => setOpen(true)}
            onMouseLeave={() => setOpen(false)}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                setOpen(false);
                return;
              }
              if (
                !["ArrowDown", "ArrowUp", "Home", "End", "PageDown", "PageUp"].includes(
                  event.key
                ) ||
                readerTotal <= 0
              ) {
                return;
              }
              const focusedIndex = readerRows.findIndex(
                (row) => readerRowRefs.current.get(row.user_id) === document.activeElement
              );
              const currentIndex =
                focusedIndex >= 0 ? readerStart + focusedIndex : readerStart;
              const pageSize = Math.max(readerRows.length, 1);
              const targetIndex =
                event.key === "ArrowDown"
                  ? currentIndex + 1
                  : event.key === "ArrowUp"
                    ? currentIndex - 1
                    : event.key === "Home"
                      ? 0
                      : event.key === "End"
                        ? readerTotal - 1
                        : event.key === "PageDown"
                          ? currentIndex + pageSize
                          : currentIndex - pageSize;
              event.preventDefault();
              focusReaderAt(Math.max(0, Math.min(targetIndex, readerTotal - 1)));
            }}
            onScroll={(event) => {
              if (!readerRevision || readerStart + readerRows.length >= readerTotal) return;
              const element = event.currentTarget;
              if (element.scrollTop + element.clientHeight < element.scrollHeight - 24) return;
              requestReaderWindowAt(readerStart + readerRows.length);
            }}
          >
            <button
              type="button"
              className="receipt-reader-close"
              aria-label={t("shortcut.closeDialogOrMenu")}
              onClick={() => setOpen(false)}
            >
              ×
            </button>
            <span role="list" aria-label={receiptLabel}>
              {readerState === "loading" && readerRows.length === 0 ? (
                <span role="status">{t("timeline.loading")}</span>
              ) : readerState === "failed" ? (
                <span role="status">{t("timeline.readReceiptFailed")}</span>
              ) : (
                <>
                  {readerRows.map((row, index) => (
                    <span
                      ref={(node) => {
                        if (node) readerRowRefs.current.set(row.user_id, node);
                        else readerRowRefs.current.delete(row.user_id);
                      }}
                      className="receipt-reader-row"
                      key={row.user_id}
                      role="listitem"
                      tabIndex={
                        focusedReaderUserId === row.user_id ||
                        (focusedReaderUserId === null && index === 0)
                          ? 0
                          : -1
                      }
                      aria-setsize={readerTotal}
                      aria-posinset={readerStart + index + 1}
                      onFocus={() => setFocusedReaderUserId(row.user_id)}
                    >
                      <EntityAvatar
                        avatar={readerAvatarImage(row.avatar)}
                        className="receipt-reader-avatar"
                        colorSeed={row.user_id}
                        fallback={row.initials}
                        sourceUrl={
                          readerInstalledRevision && row.avatar?.kind === "ready"
                            ? readerResourceUrls[row.avatar.source_ref] ?? null
                            : undefined
                        }
                      />
                      <span className="receipt-reader-name" dir="auto" title={row.display_label}>
                        {row.display_label}
                      </span>
                      {row.timestamp ? (
                        <span className="receipt-reader-time">
                          {formatReceiptTimestamp(Number(row.timestamp.unix_ms), row.timestamp.locale)}
                        </span>
                      ) : null}
                    </span>
                  ))}
                  {readerRemainingCount > 0 ? (
                    <span className="receipt-reader-row" dir="auto">
                      {t("timeline.readReceiptOverflow", { count: readerRemainingCount })}
                    </span>
                  ) : null}
                </>
              )}
            </span>
          </span>
        </FloatingLayer>
      ) : null}
    </div>
  );
}

/** Surface the reader popup must stay inside. */
function receiptPopupBoundaryElement(anchor: Element): Element | null {
  return anchor.closest(".thread-pane") ?? anchor.closest(".main-pane");
}

function compactReaderRow(receipt: LiveReadReceipt): ReaderRow {
  return {
    user_id: receipt.user_id,
    display_label: receiptDisplayName(receipt),
    original_display_label: receipt.original_display_label,
    initials: receiptInitials(receipt),
    timestamp: receipt.timestamp_ms === null ? null : {
      unix_ms: String(receipt.timestamp_ms),
      locale: getActiveLocale() === "ja" ? "ja" : "en"
    },
    avatar: receipt.avatar?.thumbnail ?? null
  };
}

function readerAvatarImage(avatar: ReaderRow["avatar"]): LiveReadReceipt["avatar"] {
  return avatar ? { mxc_uri: "", thumbnail: avatar } : null;
}

function formatReaderRow(row: ReaderRow): string {
  const timestamp = row.timestamp
    ? formatReceiptTimestamp(Number(row.timestamp.unix_ms), row.timestamp.locale)
    : null;
  return timestamp ? `${row.display_label} ${timestamp}` : row.display_label;
}

export function receiptDisplayName(receipt: LiveReadReceipt): string {
  return peopleFacingLabel(receipt.display_name, receipt.original_display_label);
}

function receiptInitials(receipt: LiveReadReceipt): string {
  const label = receiptDisplayName(receipt);
  const ascii = label.match(/[A-Za-z]/g);
  if (ascii?.length) {
    return ascii.slice(0, 2).join("").toUpperCase();
  }
  return label.slice(0, 2);
}

function createReaderResourceUrl(
  bytes: number[],
  mimeType: string | null
): { url: string; revoke: (() => void) | null } | null {
  const payload = Uint8Array.from(bytes);
  const type = mimeType || "application/octet-stream";
  if (typeof URL.createObjectURL === "function") {
    const url = URL.createObjectURL(new Blob([payload], { type }));
    return { url, revoke: () => URL.revokeObjectURL(url) };
  }
  if (typeof globalThis.btoa !== "function") return null;
  let binary = "";
  for (const byte of payload) binary += String.fromCharCode(byte);
  return { url: `data:${type};base64,${globalThis.btoa(binary)}`, revoke: null };
}

function formatReceiptTimestamp(timestampMs: number | null, locale: "en" | "ja"): string | null {
  if (timestampMs === null) {
    return null;
  }
  return new Intl.DateTimeFormat(locale, {
    dateStyle: "medium",
    timeStyle: "short"
  }).format(new Date(timestampMs));
}
