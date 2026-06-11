import {
  AtSign,
  Bell,
  Bold,
  ChevronDown,
  Clock3,
  Code2,
  Edit3,
  FileText,
  Hash,
  Headphones,
  HelpCircle,
  Home,
  Italic,
  Link2,
  List,
  MessageCircle,
  MoreHorizontal,
  MoreVertical,
  PanelRightClose,
  PanelRightOpen,
  Paperclip,
  Plus,
  Search,
  Send,
  Settings,
  Smile,
  Star,
  Users,
  X
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { createDesktopApi } from "./backend/client";
import type {
  DesktopSnapshot,
  RoomListItem,
  SearchResult,
  SearchScopeKind,
  TimelineMessage
} from "./domain/types";

const api = createDesktopApi();

export function App() {
  const [snapshot, setSnapshot] = useState<DesktopSnapshot | null>(null);
  const [searchQuery, setSearchQuery] = useState(() => initialSearchQuery());
  const [searchScope, setSearchScope] = useState<SearchScopeKind>("allRooms");
  const [composerDraft, setComposerDraft] = useState("");
  const [isBusy, setIsBusy] = useState(false);
  const searchTimer = useRef<number | null>(null);

  useEffect(() => {
    void refresh();
  }, []);

  useEffect(() => {
    if (!snapshot) {
      return;
    }

    if (searchTimer.current) {
      window.clearTimeout(searchTimer.current);
    }

    searchTimer.current = window.setTimeout(() => {
      void runSearch(searchQuery, searchScope);
    }, 120);

    return () => {
      if (searchTimer.current) {
        window.clearTimeout(searchTimer.current);
      }
    };
  }, [
    searchQuery,
    searchScope,
    snapshot?.state.navigation.active_room_id,
    snapshot?.state.navigation.active_space_id
  ]);

  async function refresh() {
    setIsBusy(true);
    try {
      setSnapshot(await api.getSnapshot());
    } finally {
      setIsBusy(false);
    }
  }

  async function selectSpace(spaceId: string | null) {
    setSnapshot(await api.selectSpace(spaceId));
  }

  async function selectRoom(roomId: string) {
    setSnapshot(await api.selectRoom(roomId));
  }

  async function openThread(roomId: string, rootEventId: string) {
    setSnapshot(await api.openThread(roomId, rootEventId));
  }

  async function closeThread() {
    setSnapshot(await api.closeThread());
  }

  async function runSearch(query: string, scope: SearchScopeKind) {
    const trimmed = query.trim();
    if (!trimmed) {
      setSnapshot(await api.getSnapshot());
      return;
    }
    setSnapshot(await api.submitSearch(trimmed, scope));
  }

  if (!snapshot) {
    return <div className="boot-screen">matrix-desktop</div>;
  }

  const activeRoom = snapshot.state.rooms.find(
    (room) => room.room_id === snapshot.state.navigation.active_room_id
  );
  const activeSpace = snapshot.state.spaces.find(
    (space) => space.space_id === snapshot.state.navigation.active_space_id
  );
  const searchResults = snapshot.state.search.kind === "results" ? snapshot.state.search.results : [];

  return (
    <div className="desktop">
      <TopBar
        activeSpaceName={activeSpace?.display_name ?? "Matrix"}
        isBusy={isBusy}
        searchQuery={searchQuery}
        searchScope={searchScope}
        onSearchQueryChange={setSearchQuery}
        onSearchScopeChange={setSearchScope}
      />
      <div className={`app-grid ${snapshot.thread ? "" : "thread-closed"}`}>
        <WorkspaceRail snapshot={snapshot} onSelectSpace={selectSpace} />
        <Sidebar
          activeRoomId={snapshot.state.navigation.active_room_id}
          snapshot={snapshot}
          onSelectRoom={selectRoom}
        />
        <TimelinePane
          activeRoomName={activeRoom?.display_name ?? "No room"}
          composerDraft={composerDraft}
          searchQuery={searchQuery}
          searchResults={searchResults}
          snapshot={snapshot}
          onComposerDraftChange={setComposerDraft}
          onOpenThread={openThread}
          onResultSelect={(roomId, eventId) => {
            void selectRoom(roomId).then(() => {
              setSearchQuery("");
              requestAnimationFrame(() => {
                document.querySelector(`[data-event-id="${cssEscape(eventId)}"]`)?.scrollIntoView({
                  block: "center"
                });
              });
            });
          }}
          onToggleThread={() => {
            if (snapshot.thread) {
              void closeThread();
            } else {
              const messageWithReplies = snapshot.timeline.find((message) => message.reply_count > 0);
              if (messageWithReplies) {
                void openThread(messageWithReplies.room_id, messageWithReplies.event_id);
              }
            }
          }}
        />
        <ThreadPane
          snapshot={snapshot}
          searchQuery={searchQuery}
          onCloseThread={() => {
            void closeThread();
          }}
        />
      </div>
    </div>
  );
}

function TopBar({
  activeSpaceName,
  isBusy,
  searchQuery,
  searchScope,
  onSearchQueryChange,
  onSearchScopeChange
}: {
  activeSpaceName: string;
  isBusy: boolean;
  searchQuery: string;
  searchScope: SearchScopeKind;
  onSearchQueryChange: (value: string) => void;
  onSearchScopeChange: (value: SearchScopeKind) => void;
}) {
  return (
    <header className="titlebar">
      <div className="traffic">
        <span className="dot red" />
        <span className="dot yellow" />
        <span className="dot green" />
      </div>
      <div className="history">
        <button className="icon-button" type="button" aria-label="Back">
          ‹
        </button>
        <button className="icon-button" type="button" aria-label="Forward">
          ›
        </button>
        <button className="icon-button" type="button" aria-label="History">
          <Clock3 size={18} />
        </button>
      </div>
      <label className="top-search">
        <Search size={17} />
        <input
          value={searchQuery}
          placeholder={`${activeSpaceName} 内を検索する`}
          onChange={(event) => onSearchQueryChange(event.target.value)}
        />
      </label>
      <select
        className="scope-select"
        aria-label="Search scope"
        value={searchScope}
        onChange={(event) => onSearchScopeChange(event.target.value as SearchScopeKind)}
      >
        <option value="allRooms">All</option>
        <option value="currentSpace">Space</option>
        <option value="currentRoom">Room</option>
        <option value="dms">DM</option>
      </select>
      <div className="top-actions">
        <span className={`sync-dot ${isBusy ? "busy" : ""}`} />
        <button className="icon-button" type="button" aria-label="Help">
          <HelpCircle size={18} />
        </button>
      </div>
    </header>
  );
}

function WorkspaceRail({
  snapshot,
  onSelectSpace
}: {
  snapshot: DesktopSnapshot;
  onSelectSpace: (spaceId: string) => void;
}) {
  return (
    <nav className="workspace-rail" aria-label="Workspaces">
      <div className="workspace-list">
        {snapshot.sidebar.space_rail.map((space) => (
          <button
            className={`workspace-button ${space.is_active ? "is-active" : ""}`}
            data-count={space.unread_count || undefined}
            key={space.space_id}
            type="button"
            aria-label={space.display_name}
            onClick={() => onSelectSpace(space.space_id)}
          >
            {initials(space.display_name)}
          </button>
        ))}
      </div>
      <div className="rail-footer">
        <button className="rail-action" type="button" aria-label="Add workspace">
          <Plus size={22} />
        </button>
        <div className="user-presence" aria-label="Online" />
      </div>
    </nav>
  );
}

function Sidebar({
  activeRoomId,
  snapshot,
  onSelectRoom
}: {
  activeRoomId: string | null;
  snapshot: DesktopSnapshot;
  onSelectRoom: (roomId: string) => void;
}) {
  return (
    <aside className="sidebar">
      <div className="workspace-header">
        <div className="workspace-name">
          {snapshot.sidebar.space_rail.find((space) => space.is_active)?.display_name ?? "Matrix"}
        </div>
        <button className="icon-button" type="button" aria-label="Preferences">
          <Settings size={18} />
        </button>
        <button className="icon-button" type="button" aria-label="New message">
          <Edit3 size={18} />
        </button>
      </div>
      <button className="upgrade" type="button">
        <Star size={18} />
        <span>プランをアップグレード</span>
      </button>
      <div className="sidebar-scroll">
        <NavButton icon={<Home size={18} />} label="ホーム" />
        <NavButton icon={<MessageCircle size={18} />} label="スレッド" />
        <NavButton icon={<Headphones size={18} />} label="ハドルミーティング" />
        <NavButton icon={<Bell size={18} />} label="下書き & 送信済み" />
        <SectionTitle label="チャンネル" />
        {snapshot.sidebar.space_rooms.map((room) => (
          <RoomButton
            activeRoomId={activeRoomId}
            icon={<Hash size={16} />}
            key={room.room_id}
            room={room}
            onSelectRoom={onSelectRoom}
          />
        ))}
        <SectionTitle label="ダイレクトメッセージ" />
        {snapshot.sidebar.global_dms.map((room) => (
          <RoomButton
            activeRoomId={activeRoomId}
            icon={<span className="presence-dot" />}
            key={room.room_id}
            room={room}
            onSelectRoom={onSelectRoom}
          />
        ))}
        <SectionTitle label="App" />
        <NavButton icon={<FileText size={17} />} label="Slackbot" />
      </div>
    </aside>
  );
}

function NavButton({ icon, label }: { icon: React.ReactNode; label: string }) {
  return (
    <button className="nav-item" type="button">
      {icon}
      <span className="nav-label">{label}</span>
    </button>
  );
}

function SectionTitle({ label }: { label: string }) {
  return (
    <div className="section-title">
      <span>{label}</span>
      <Plus size={15} />
    </div>
  );
}

function RoomButton({
  activeRoomId,
  icon,
  room,
  onSelectRoom
}: {
  activeRoomId: string | null;
  icon: React.ReactNode;
  room: RoomListItem;
  onSelectRoom: (roomId: string) => void;
}) {
  return (
    <button
      className={`room-item ${room.room_id === activeRoomId ? "is-active" : ""}`}
      type="button"
      onClick={() => onSelectRoom(room.room_id)}
    >
      {icon}
      <span className="room-name">{room.display_name}</span>
      <span className="room-count">{room.unread_count || ""}</span>
    </button>
  );
}

function TimelinePane({
  activeRoomName,
  composerDraft,
  searchQuery,
  searchResults,
  snapshot,
  onComposerDraftChange,
  onOpenThread,
  onResultSelect,
  onToggleThread
}: {
  activeRoomName: string;
  composerDraft: string;
  searchQuery: string;
  searchResults: SearchResult[];
  snapshot: DesktopSnapshot;
  onComposerDraftChange: (value: string) => void;
  onOpenThread: (roomId: string, rootEventId: string) => void;
  onResultSelect: (roomId: string, eventId: string) => void;
  onToggleThread: () => void;
}) {
  return (
    <main className="main-pane">
      <header className="channel-header">
        <div className="channel-title">
          <Hash size={22} />
          <span>{activeRoomName}</span>
        </div>
        <div className="channel-actions">
          <button className="member-pill" type="button" aria-label="Members">
            <Users size={16} />
            <span>8</span>
          </button>
          <button className="icon-button" type="button" aria-label="Toggle thread" onClick={onToggleThread}>
            {snapshot.thread ? <PanelRightClose size={19} /> : <PanelRightOpen size={19} />}
          </button>
          <button className="icon-button" type="button" aria-label="More">
            <MoreVertical size={19} />
          </button>
        </div>
      </header>
      <nav className="tabs" aria-label="Room tabs">
        <button className="tab is-active" type="button">
          メッセージ
        </button>
        <button className="tab" type="button">
          canvas を追加する
        </button>
        <button className="tab" type="button">
          その他 <ChevronDown size={14} />
        </button>
        <button className="tab" type="button" aria-label="Add tab">
          <Plus size={17} />
        </button>
      </nav>
      <section className="timeline-scroll">
        <SearchResults
          query={searchQuery}
          results={searchResults}
          rooms={snapshot.state.rooms}
          onResultSelect={onResultSelect}
        />
        <div className="message-list">
          {snapshot.timeline.map((message) => (
            <MessageArticle
              key={message.event_id}
              message={message}
              query={searchQuery}
              onOpenThread={onOpenThread}
            />
          ))}
        </div>
      </section>
      <Composer
        roomName={activeRoomName}
        value={composerDraft}
        onValueChange={onComposerDraftChange}
      />
    </main>
  );
}

function SearchResults({
  query,
  results,
  rooms,
  onResultSelect
}: {
  query: string;
  results: SearchResult[];
  rooms: DesktopSnapshot["state"]["rooms"];
  onResultSelect: (roomId: string, eventId: string) => void;
}) {
  if (!query.trim()) {
    return null;
  }

  return (
    <section className="search-results">
      <div className="search-results-header">
        <span>
          {results.length} result{results.length === 1 ? "" : "s"} for "{query}"
        </span>
      </div>
      <div className="result-list">
        {results.length ? (
          results.map((result) => {
            const room = rooms.find((candidate) => candidate.room_id === result.room_id);
            return (
              <button
                className="result-button"
                key={`${result.room_id}:${result.event_id}`}
                type="button"
                onClick={() => onResultSelect(result.room_id, result.event_id)}
              >
                <span>{highlight(result.snippet, result.highlights)}</span>
                <span className="result-meta">
                  {room?.display_name ?? result.room_id} · {matchFieldLabel(result.match_field)}
                </span>
              </button>
            );
          })
        ) : (
          <div className="empty-results">No exact matches</div>
        )}
      </div>
    </section>
  );
}

function MessageArticle({
  message,
  query,
  onOpenThread
}: {
  message: TimelineMessage;
  query: string;
  onOpenThread: (roomId: string, rootEventId: string) => void;
}) {
  return (
    <article className="message" data-event-id={message.event_id}>
      <div className={`avatar ${message.sender === "Slackbot" ? "bot" : ""}`} aria-hidden="true">
        {initials(message.sender)}
      </div>
      <div className="message-main">
        <div className="message-heading">
          <span className="sender">{message.sender}</span>
          <span className="time">{formatTime(message.timestamp_ms)}</span>
        </div>
        <div className="message-body">{highlightQueryLines(message.body, query)}</div>
        {message.attachment_filename ? (
          <div className="attachment">
            <Paperclip size={16} />
            <span>{highlightQueryLines(message.attachment_filename, query)}</span>
          </div>
        ) : null}
        {message.reply_count ? (
          <button
            className="reply-link"
            type="button"
            onClick={() => onOpenThread(message.room_id, message.event_id)}
          >
            新しい返信を確認する · {message.reply_count}
          </button>
        ) : null}
      </div>
    </article>
  );
}

function Composer({
  roomName,
  value,
  onValueChange
}: {
  roomName: string;
  value: string;
  onValueChange: (value: string) => void;
}) {
  return (
    <section className="composer" aria-label="Message composer">
      <div className="composer-tools">
        <button className="icon-button" type="button" aria-label="Bold">
          <Bold size={17} />
        </button>
        <button className="icon-button" type="button" aria-label="Italic">
          <Italic size={17} />
        </button>
        <button className="icon-button" type="button" aria-label="Link">
          <Link2 size={17} />
        </button>
        <button className="icon-button" type="button" aria-label="List">
          <List size={17} />
        </button>
        <button className="icon-button" type="button" aria-label="Code">
          <Code2 size={17} />
        </button>
      </div>
      <textarea
        value={value}
        placeholder={`${roomName} へのメッセージ`}
        onChange={(event) => onValueChange(event.target.value)}
      />
      <div className="composer-footer">
        <div>
          <button className="icon-button" type="button" aria-label="Add">
            <Plus size={19} />
          </button>
          <button className="icon-button" type="button" aria-label="Mention">
            <AtSign size={18} />
          </button>
          <button className="icon-button" type="button" aria-label="Emoji">
            <Smile size={18} />
          </button>
        </div>
        <button className={`send-button ${value.trim() ? "ready" : ""}`} type="button" aria-label="Send">
          <Send size={17} />
        </button>
      </div>
    </section>
  );
}

function ThreadPane({
  snapshot,
  searchQuery,
  onCloseThread
}: {
  snapshot: DesktopSnapshot;
  searchQuery: string;
  onCloseThread: () => void;
}) {
  if (!snapshot.thread) {
    return <aside className="thread-pane" />;
  }

  const root = snapshot.timeline.find((message) => message.event_id === snapshot.thread?.root_event_id);

  return (
    <aside className="thread-pane">
      <header className="thread-header">
        <div className="thread-title">スレッド</div>
        <button className="icon-button" type="button" aria-label="More">
          <MoreHorizontal size={19} />
        </button>
        <button className="icon-button" type="button" aria-label="Close thread" onClick={onCloseThread}>
          <X size={19} />
        </button>
      </header>
      <section className="thread-scroll">
        {root ? (
          <div className="thread-root">
            <MessageArticle message={root} query={searchQuery} onOpenThread={() => undefined} />
          </div>
        ) : null}
        {snapshot.thread.replies.map((reply) => (
          <article className="thread-reply" key={reply.event_id}>
            <div className="avatar" aria-hidden="true">
              {initials(reply.sender)}
            </div>
            <div className="message-main">
              <div className="message-heading">
                <span className="sender">{reply.sender}</span>
                <span className="time">{formatTime(reply.timestamp_ms)}</span>
              </div>
              <div className="message-body">{reply.body}</div>
            </div>
          </article>
        ))}
      </section>
      <section className="thread-composer" aria-label="Thread composer">
        <textarea placeholder="返信する..." />
      </section>
    </aside>
  );
}

function highlightQueryLines(text: string, query: string) {
  if (!query.trim()) {
    return text.split("\n").map((line, index) => (
      <span key={`${line}:${index}`}>
        {index > 0 ? <br /> : null}
        {line}
      </span>
    ));
  }

  return text.split("\n").map((line, index) => (
    <span key={`${line}:${index}`}>
      {index > 0 ? <br /> : null}
      {highlightString(line, query)}
    </span>
  ));
}

function highlightString(text: string, query: string) {
  const index = text.indexOf(query);
  if (index < 0 || query.length === 0) {
    return text;
  }
  return (
    <>
      {text.slice(0, index)}
      <mark>{text.slice(index, index + query.length)}</mark>
      {text.slice(index + query.length)}
    </>
  );
}

function highlight(text: string, ranges: SearchResult["highlights"]) {
  if (!ranges.length) {
    return text;
  }

  const range = ranges[0];
  const chars = Array.from(text);
  const start = utf16OffsetToCodePointIndex(text, range.start_utf16);
  const end = utf16OffsetToCodePointIndex(text, range.end_utf16);
  return (
    <>
      {chars.slice(0, start).join("")}
      <mark>{chars.slice(start, end).join("")}</mark>
      {chars.slice(end).join("")}
    </>
  );
}

function utf16OffsetToCodePointIndex(value: string, offset: number): number {
  let utf16Count = 0;
  for (const [index, char] of Array.from(value).entries()) {
    if (utf16Count >= offset) {
      return index;
    }
    utf16Count += char.length;
  }
  return Array.from(value).length;
}

function matchFieldLabel(field: SearchResult["match_field"]): string {
  switch (field) {
    case "messageBody":
      return "message";
    case "attachmentFileName":
      return "attachment filename";
  }
}

function initials(value: string): string {
  const ascii = value.match(/[A-Za-z]/g);
  if (ascii?.length) {
    return ascii.slice(0, 2).join("").toUpperCase();
  }
  return value.slice(0, 2);
}

function formatTime(timestampMs: number): string {
  return new Intl.DateTimeFormat("ja-JP", {
    hour: "2-digit",
    minute: "2-digit"
  }).format(new Date(timestampMs));
}

function cssEscape(value: string): string {
  return value.replace(/["\\]/g, "\\$&");
}

function initialSearchQuery(): string {
  return new URLSearchParams(window.location.search).get("q") ?? "";
}
