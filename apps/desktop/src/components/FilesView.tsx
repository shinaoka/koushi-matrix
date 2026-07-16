import {
  FileAudio,
  FileText,
  Image as ImageIcon,
  Smile,
  Video
} from "lucide-react";
import { useState } from "react";

import { t } from "../i18n/messages";
import { ImeTextField } from "./ImeTextControl";
import type {
  AttachmentFilter,
  AttachmentKind,
  AttachmentResult,
  AttachmentScope,
  AttachmentSort,
  FilesViewState
} from "../domain/types";

const ALL_KINDS: AttachmentKind[] = ["image", "video", "audio", "file", "sticker"];

const KIND_ICONS: Record<AttachmentKind, typeof FileText> = {
  image: ImageIcon,
  video: Video,
  audio: FileAudio,
  file: FileText,
  sticker: Smile
};

export interface FilesViewProps {
  filesView: FilesViewState;
  onChangeFilterSort: (scope: AttachmentScope, filter: AttachmentFilter, sort: AttachmentSort) => void;
}

export function FilesView({ filesView, onChangeFilterSort }: FilesViewProps) {
  if (filesView.kind === "closed") {
    return null;
  }

  const toolbarProps =
    filesView.kind === "open" || filesView.kind === "loading"
      ? {
          scope: filesView.scope,
          filter: filesView.filter,
          sort: filesView.sort
        }
      : null;

  return (
    <section className="files-view-panel" aria-label={t("files.title")}>
      {toolbarProps ? (
        <FilesViewToolbar
          scope={toolbarProps.scope}
          filter={toolbarProps.filter}
          sort={toolbarProps.sort}
          onChangeFilterSort={onChangeFilterSort}
        />
      ) : null}
      {filesView.kind === "loading" ? (
        <div className="files-view-empty">{t("files.loading")}</div>
      ) : filesView.kind === "failed" ? (
        <div className="files-view-empty files-view-error">{t("files.error")}</div>
      ) : filesView.items.length === 0 ? (
        <div className="files-view-empty">{t("files.empty")}</div>
      ) : (
        <ul className="files-view-list" role="listbox" aria-label={t("files.title")}>
          {filesView.items.map((item) => (
            <FilesViewRow key={item.event_id} item={item} />
          ))}
        </ul>
      )}
    </section>
  );
}

function FilesViewToolbar({
  scope,
  filter,
  sort,
  onChangeFilterSort
}: {
  scope: AttachmentScope;
  filter: AttachmentFilter;
  sort: AttachmentSort;
  onChangeFilterSort: (scope: AttachmentScope, filter: AttachmentFilter, sort: AttachmentSort) => void;
}) {
  const [draftQuery, setDraftQuery] = useState(filter.filename_query ?? "");

  function apply(nextFilter: AttachmentFilter, nextSort: AttachmentSort) {
    onChangeFilterSort(scope, nextFilter, nextSort);
  }

  function toggleKind(kind: AttachmentKind) {
    const nextKinds = filter.kinds.includes(kind)
      ? filter.kinds.filter((k) => k !== kind)
      : [...filter.kinds, kind];
    apply({ ...filter, kinds: nextKinds.length ? nextKinds : ALL_KINDS }, sort);
  }

  function applyQuery() {
    apply({ ...filter, filename_query: draftQuery.trim() || null }, sort);
  }

  return (
    <div className="files-view-toolbar">
      <div className="files-view-kind-filters" role="group" aria-label={t("files.filterKinds")}>
        {ALL_KINDS.map((kind) => (
          <label key={kind} className="files-view-kind-chip">
            <input
              type="checkbox"
              checked={filter.kinds.includes(kind)}
              onChange={() => toggleKind(kind)}
            />
            <span>{t(`files.kind.${kind}`)}</span>
          </label>
        ))}
      </div>
      <div className="files-view-query-row">
        <ImeTextField
          className="files-view-query-input"
          type="search"
          value={draftQuery}
          syncKey="files-query"
          placeholder={t("files.filterPlaceholder")}
          onChange={(event) => setDraftQuery(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              applyQuery();
            }
          }}
        />
        <select
          className="files-view-sort-select"
          value={sort}
          onChange={(event) => apply(filter, event.currentTarget.value as AttachmentSort)}
          aria-label={t("files.sortLabel")}
        >
          <option value="newestFirst">{t("files.sort.newestFirst")}</option>
          <option value="oldestFirst">{t("files.sort.oldestFirst")}</option>
          <option value="sender">{t("files.sort.sender")}</option>
          <option value="filename">{t("files.sort.filename")}</option>
        </select>
      </div>
    </div>
  );
}

function FilesViewRow({ item }: { item: AttachmentResult }) {
  const Icon = KIND_ICONS[item.kind];
  const size = item.size == null ? null : formatBytes(item.size);
  const date = item.timestamp_ms == null ? null : formatDate(item.timestamp_ms);

  return (
    <li className="files-view-row" role="option">
      <span className="files-view-row-icon" aria-hidden="true">
        <Icon size={18} />
      </span>
      <span className="files-view-row-main" dir="auto">
        <span className="files-view-row-name">{item.filename}</span>
        <span className="files-view-row-meta">
          {item.sender}
          {date ? ` · ${date}` : null}
          {size ? ` · ${size}` : null}
        </span>
      </span>
    </li>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  const kb = bytes / 1024;
  if (kb < 1024) {
    return `${kb.toFixed(1)} KB`;
  }
  const mb = kb / 1024;
  if (mb < 1024) {
    return `${mb.toFixed(1)} MB`;
  }
  const gb = mb / 1024;
  return `${gb.toFixed(1)} GB`;
}

function formatDate(timestampMs: number): string {
  const date = new Date(timestampMs);
  return date.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric"
  });
}
