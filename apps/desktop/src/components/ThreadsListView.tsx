import { MessageCircle } from "lucide-react";

import { t } from "../i18n/messages";
import type { ThreadsListItem, ThreadsListState } from "../domain/types";

export interface ThreadsListViewProps {
  threadsList: ThreadsListState;
  roomId: string | null;
  onClose: () => void;
  onOpenThread: (roomId: string, rootEventId: string) => void;
  onPaginate: (roomId: string) => void;
}

export function ThreadsListView({
  threadsList,
  roomId,
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
          <ul className="threads-list" role="listbox" aria-label={t("threads.title")}>
            {threadsList.items.map((item) => (
              <ThreadsListRow
                key={item.root_event_id}
                item={item}
                onClick={() => {
                  if (roomId) {
                    onOpenThread(roomId, item.root_event_id);
                  }
                }}
              />
            ))}
          </ul>
          {!threadsList.end_reached && !threadsList.is_paginating && roomId ? (
            <button
              className="threads-list-load-more"
              type="button"
              onClick={() => onPaginate(roomId)}
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
    <li className="threads-list-row" role="option">
      <button className="threads-list-row-button" type="button" onClick={onClick}>
        <span className="threads-list-row-icon" aria-hidden="true">
          <MessageCircle size={18} />
        </span>
        <span className="threads-list-row-main" dir="auto">
          <span className="threads-list-row-preview">
            {item.root_body_preview ?? t("activity.noPreview")}
          </span>
          <span className="threads-list-row-meta">
            {item.root_sender_label ?? item.root_sender}
            {item.latest_body_preview ? (
              <>
                {" · "}
                {item.latest_sender_label ?? item.latest_sender ?? " "}: {item.latest_body_preview}
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
