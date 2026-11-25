import { createTrackedObjectURL, revokeTrackedObjectURL } from "../../domain/objectUrlDiagnostics";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useActiveLocaleKey } from "../../i18n/activeLocaleContext";
import { getActiveLocale, localeLanguageTag, t } from "../../i18n/messages";
import { avatarInitial, peopleFacingLabel } from "../../app/uiShared";
import {
  containsTarget,
  PersonListPopover,
  personListBoundaryElement,
  type PersonListRow
} from "./PersonListPopover";
import { timelineKeyIdentity, type ReceiptSourceRef, type ReaderRow } from "../../domain/coreEvents";
import { EntityAvatar } from "../Shell";
import { api as defaultApi } from "../../backend/appRuntime";
import type { LiveReadReceipt } from "../../domain/types";
import { cachedDateTimeFormat } from "../../domain/intlFormatCache";

/**
 * Read-receipt avatar stack plus its reader popup.
 *
 * The popup renders in the body-level floating layer: the thread pane is
 * overflow-clipped, so a row-local popup gets cut off at the pane edge. Hover
 * and focus open the same popup so keyboard users reach what pointer users see.
 */
export function ReceiptReaders({
  accountTabId,
  overflowCount,
  receipts,
  source,
  totalCount,
  onRequestAvatarThumbnail
}: {
  accountTabId?: string;
  overflowCount: number;
  receipts: LiveReadReceipt[];
  source?: ReceiptSourceRef;
  totalCount: number;
  onRequestAvatarThumbnail?: (mxcUri: string) => void | Promise<void | (() => void)>;
}) {
  const receiptApi = useMemo(
    () =>
      accountTabId === undefined
        ? defaultApi
        : defaultApi.forAccountTab?.(accountTabId) ?? defaultApi,
    [accountTabId]
  );
  const anchorRef = useRef<HTMLDivElement>(null);
  const popupRef = useRef<HTMLSpanElement>(null);
  const [open, setOpen] = useState(false);
  const [readerScope, setReaderScope] = useState<string | null>(null);
  const [readerRevision, setReaderRevision] = useState<string | null>(null);
  const [readerInstalledRevision, setReaderInstalledRevision] = useState<string | null>(null);
  const [readerResourceUrls, setReaderResourceUrls] = useState<Record<string, string>>({});
  const [readerStart, setReaderStart] = useState(0);
  const [readerTotal, setReaderTotal] = useState(0);
  const [readerState, setReaderState] = useState<"loading" | "ready" | "failed">("loading");
  // #1557: the rows capture the catalog locale for their timestamps, so a
  // locale switch must rebuild them.
  const localeKey = useActiveLocaleKey();
  const compactRows = useMemo(() => receipts.map(compactReaderRow), [receipts, localeKey]);
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
  const readerRowRefs = useRef(new Map<string, HTMLElement>());
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
    (start: number) => {
      if (!readerScope || !readerRevision || readerTotal <= 0) return;
      const nextStart = Math.max(0, Math.min(Math.trunc(start), readerTotal - 1));
      const sequence = String(++readerSequenceRef.current);
      void receiptApi.updateReceiptReaderWindow(readerScope, {
        installed_revision: readerRevision,
        sequence,
        target: { kind: "index", start: String(nextStart) },
        limit: 256
      });
    },
    [readerRevision, readerScope, readerTotal, receiptApi]
  );
  useEffect(() => {
    if (!open) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented && !event.isComposing && event.keyCode !== 229) setOpen(false);
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
    void receiptApi
      .subscribeReceiptReader(activeSource, 0, 256)
      .then((nextScope) => {
        if (cancelled) {
          void receiptApi.closeReceiptReader(nextScope);
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
      if (scope) void receiptApi.closeReceiptReader(scope);
      setReaderScope(null);
      appliedReaderRevisionRef.current = null;
      readerSequenceRef.current = 0;
      avatarObservationSequenceRef.current = 0n;
      setReaderRevision(null);
      setReaderInstalledRevision(null);
      setReaderResourceUrls({});
      setReaderStart(0);
      setReaderTotal(0);
      setReaderRows([]);
      setReaderState("loading");
    };
  }, [open, sourceIdentity, receiptApi]);
  useEffect(() => {
    if (!readerScope) return;
    let cancelled = false;
    const receive = async (): Promise<void> => {
      while (!cancelled) {
        const delivery = await receiptApi.receiveReceiptReader(readerScope);
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
        setReaderRows(delivery.model.rows);
        await receiptApi.ackReceiptReader(readerScope, delivery.revision);
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
  }, [readerScope, receiptApi]);
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
          const content = await receiptApi.readReceiptReaderResource(
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
        for (const url of ownedObjectUrls) revokeTrackedObjectURL(url);
        return;
      }
      setReaderResourceUrls(Object.fromEntries(entries));
    };
    void loadResources();
    return () => {
      cancelled = true;
      for (const url of ownedObjectUrls) revokeTrackedObjectURL(url);
    };
  }, [readerInstalledRevision, readerResourceIdentity, readerScope, receiptApi]);
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
      void receiptApi.observeReceiptReaderAvatars(readerScope, {
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
  }, [open, readerInstalledRevision, readerRevision, readerRows, readerScope, receiptApi]);
  const readerRemainingCount = source
    ? Math.max(readerTotal - readerStart - readerRows.length, 0)
    : Math.max(overflowCount, 0);
  const personRows = useMemo<PersonListRow[]>(
    () =>
      readerRows.map((row) => ({
        user_id: row.user_id,
        display_name: row.display_label,
        initials: row.initials,
        avatar: readerAvatarImage(row.avatar),
        source_url:
          readerInstalledRevision && row.avatar?.kind === "ready"
            ? readerResourceUrls[row.avatar.source_ref] ?? null
            : undefined,
        meta: row.timestamp
          ? formatReceiptTimestamp(Number(row.timestamp.unix_ms), row.timestamp.locale)
          : null
      })),
    [readerInstalledRevision, readerResourceUrls, readerRows]
  );
  // Until a committed source exists the bounded summary is all there is; with
  // one, an empty ready window has nothing to show and stays closed (#2098).
  const showPopover = open && (readerRows.length > 0 || readerState !== "ready");

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
        if (event.key === "Escape" && !event.nativeEvent.isComposing && event.keyCode !== 229) {
          event.preventDefault(); event.stopPropagation(); setOpen(false); return;
        }
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
            fallback={avatarInitial(receiptDisplayName(receipt))}
            key={receipt.user_id}
            onRequestAvatarThumbnail={onRequestAvatarThumbnail}
          />
        ))}
        {overflowCount > 0 ? <span className="receipt-overflow">+{overflowCount}</span> : null}
      </span>
      <PersonListPopover
        anchorRef={anchorRef}
        busy={readerState === "loading"}
        dismissOnPointerLeave
        failed={readerState === "failed"}
        failedText={t("timeline.readReceiptFailed")}
        label={receiptLabel}
        loadingText={t("timeline.loading")}
        open={showPopover}
        overflowCount={readerRemainingCount}
        overflowText={t("timeline.readReceiptOverflow", { count: readerRemainingCount })}
        popupRef={popupRef}
        resolveBoundaryElement={personListBoundaryElement}
        rowElements={readerRowRefs.current}
        rows={personRows}
        startIndex={readerStart}
        totalCount={readerTotal}
        onClose={() => setOpen(false)}
        onFocusIndex={(index) => {
          // A row outside the loaded window needs its own bounded request; the
          // last row asks for the final window so `End` lands on it.
          const requestStart =
            index >= readerTotal - 1 ? Math.max(readerTotal - 256, 0) : index;
          requestReaderWindowAt(requestStart);
        }}
        onScrollNearEnd={() => {
          if (!readerRevision || readerStart + readerRows.length >= readerTotal) return;
          requestReaderWindowAt(readerStart + readerRows.length);
        }}
      />
    </div>
  );
}

function compactReaderRow(receipt: LiveReadReceipt): ReaderRow {
  return {
    user_id: receipt.user_id,
    display_label: receiptDisplayName(receipt),
    original_display_label: receipt.original_display_label,
    initials: avatarInitial(receiptDisplayName(receipt)),
    timestamp: receipt.timestamp_ms === null ? null : {
      unix_ms: String(receipt.timestamp_ms),
      locale: localeLanguageTag(getActiveLocale())
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

function createReaderResourceUrl(
  bytes: number[],
  mimeType: string | null
): { url: string; revoke: (() => void) | null } | null {
  const payload = Uint8Array.from(bytes);
  const type = mimeType || "application/octet-stream";
  if (typeof URL.createObjectURL === "function") {
    const url = createTrackedObjectURL(new Blob([payload], { type }));
    return { url, revoke: () => revokeTrackedObjectURL(url) };
  }
  if (typeof globalThis.btoa !== "function") return null;
  let binary = "";
  for (const byte of payload) binary += String.fromCharCode(byte);
  return { url: `data:${type};base64,${globalThis.btoa(binary)}`, revoke: null };
}

function formatReceiptTimestamp(
  timestampMs: number | null,
  locale: NonNullable<ReaderRow["timestamp"]>["locale"]
): string | null {
  if (timestampMs === null) {
    return null;
  }
  return cachedDateTimeFormat(locale, {
    dateStyle: "medium",
    timeStyle: "short"
  }).format(new Date(timestampMs));
}
