import { MessageCircle } from "lucide-react";

import { t } from "../i18n/messages";
import { peopleFacingLabel } from "../app/uiShared";
import type {
  ThreadOpenIntent,
  ThreadsListItem,
  ThreadsListScope,
  ThreadsListState
} from "../domain/types";

export interface ThreadsListViewProps {
  threadsList: ThreadsListState;
  scope: ThreadsListScope;
  onClose: () => void;
  onOpenThread: (
    roomId: string,
    rootEventId: string,
    intent: ThreadOpenIntent
  ) => void;
  onPaginate: (scope: ThreadsListScope) => void;
}

export function ThreadsListView({
  threadsList,
  scope,
  onOpenThread,
  onPaginate
}: ThreadsListViewProps) {
  if (threadsList.kind === "closed") {
    return null;
  }

  return (
    <section className="threads-list-panel" aria-label={t("threads.title")}>
      {threadsList.kind === "loading" ? (
        <div className="threads-list-empty">{t("threads.loading")}</div>
      ) : threadsList.kind === "failed" ? (
        <div className="threads-list-empty threads-list-error">{t("threads.error")}</div>
      ) : threadsList.items.length === 0 ? (
        <div className="threads-list-empty">{t("threads.empty")}</div>
      ) : (
        <>
          <ul className="threads-list" aria-label={t("threads.title")}>
            {threadsList.items.map((item) => (
              <ThreadsListRow
                key={`${item.room_id}:${item.root_event_id}`}
                item={item}
                onClick={() => {
                  onOpenThread(item.room_id, item.root_event_id, "existingThread");
                }}
              />
            ))}
          </ul>
          {!threadsList.end_reached && !threadsList.is_paginating ? (
            <button
              className="threads-list-load-more"
              type="button"
              onClick={() => onPaginate(scope)}
            >
              {t("activity.loadMore")}
            </button>
          ) : null}
          {threadsList.is_paginating ? (
            <div className="threads-list-empty">{t("threads.loading")}</div>
          ) : null}
        </>
      )}
    </section>
  );
}

function ThreadsListRow({
  item,
  onClick
}: {
  item: ThreadsListItem;
  onClick: () => void;
}) {
  return (
    <li className="threads-list-row">
      <button className="threads-list-row-button" type="button" onClick={onClick}>
        <span className="threads-list-row-icon" aria-hidden="true">
          <MessageCircle size={18} />
        </span>
        <span className="threads-list-row-main">
          <span className="threads-list-row-preview" dir="auto">
            {item.root_body_preview ?? t("activity.noPreview")}
          </span>
          <span className="threads-list-row-meta">
            <span dir="auto">{peopleFacingLabel(item.root_sender_label)}</span>
            {item.latest_body_preview ? (
              <>
                {" · "}
                <span dir="auto">{peopleFacingLabel(item.latest_sender_label)}</span>: {" "}
                <span dir="auto">{item.latest_body_preview}</span>
              </>
            ) : null}
            {" · "}
            {t("threads.replyCount", { count: item.reply_count })}
          </span>
        </span>
      </button>
    </li>
  );
}
