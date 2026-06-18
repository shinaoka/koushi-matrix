const spaces = [
  {
    id: "!space-alpha:example.invalid",
    shortName: "OC",
    name: "Synthetic Workspace",
    childRoomIds: ["!room-alpha:example.invalid", "!room-planning:example.invalid"],
  },
  {
    id: "!space-beta:example.invalid",
    shortName: "SL",
    name: "Synthetic Lab",
    childRoomIds: ["!room-search:example.invalid"],
  },
];

const rooms = [
  {
    id: "!room-alpha:example.invalid",
    name: "synthetic-room",
    isDm: false,
    unread: 8,
    parentSpaceIds: ["!space-alpha:example.invalid"],
  },
  {
    id: "!room-planning:example.invalid",
    name: "planning-room",
    isDm: false,
    unread: 2,
    parentSpaceIds: ["!space-alpha:example.invalid"],
  },
  {
    id: "!room-search:example.invalid",
    name: "matrix-sdk-search",
    isDm: false,
    unread: 1,
    parentSpaceIds: ["!space-beta:example.invalid"],
  },
  {
    id: "!dm-member-1:example.invalid",
    name: "Member 1",
    isDm: true,
    unread: 1,
    parentSpaceIds: [],
  },
  {
    id: "!dm-member-2:example.invalid",
    name: "Member 2",
    isDm: true,
    unread: 0,
    parentSpaceIds: [],
  },
];

const messages = [
  {
    roomId: "!room-alpha:example.invalid",
    eventId: "$alpha-update",
    sender: "Demo Coordinator",
    time: "15:04",
    body: "Alpha keyword update from demo coordinator.",
    replyCount: 2,
  },
  {
    roomId: "!room-alpha:example.invalid",
    eventId: "$agenda",
    sender: "Demo Coordinator",
    time: "15:08",
    body: "Synthetic planning note.\n\n- Fixture item one\n- Fixture item two",
    replyCount: 0,
  },
  {
    roomId: "!room-alpha:example.invalid",
    eventId: "$budget-file",
    sender: "Slackbot",
    time: "15:12",
    body: "Budget spreadsheet attached.",
    attachmentFilename: "fixture_budget.xlsx",
    replyCount: 0,
    bot: true,
  },
  {
    roomId: "!room-alpha:example.invalid",
    eventId: "$false-positive",
    sender: "Member 3",
    time: "15:16",
    body: "Non-matching synthetic note.",
    replyCount: 0,
  },
  {
    roomId: "!room-planning:example.invalid",
    eventId: "$late-original",
    sender: "Member 1",
    time: "15:22",
    body: "Final synthetic checklist",
    replyCount: 0,
  },
  {
    roomId: "!room-search:example.invalid",
    eventId: "$search-dev-note",
    sender: "Member 4",
    time: "15:28",
    body: "matrix-sdk-search adapter review notes",
    replyCount: 0,
  },
  {
    roomId: "!dm-member-1:example.invalid",
    eventId: "$dm-1",
    sender: "Member 1",
    time: "15:32",
    body: "Element mobile の event cache 側も見ておきます。",
    replyCount: 0,
  },
];

const threadReplies = [
  {
    roomId: "!room-alpha:example.invalid",
    rootEventId: "$alpha-update",
    eventId: "$thread-1",
    sender: "Member 2",
    time: "15:05",
    body: "Synthetic follow-up item one.",
  },
  {
    roomId: "!room-alpha:example.invalid",
    rootEventId: "$alpha-update",
    eventId: "$thread-2",
    sender: "Member 1",
    time: "15:07",
    body: "Synthetic follow-up item two.",
  },
];

const state = {
  activeSpaceId: "!space-alpha:example.invalid",
  activeRoomId: "!room-alpha:example.invalid",
  threadRootEventId: "$alpha-update",
  searchQuery: "",
  focusedEventId: null,
  composerDraft: "",
};

const app = document.querySelector("#app");

function render() {
  const activeRoom = roomById(state.activeRoomId);
  const activeSpace = spaces.find((space) => space.id === state.activeSpaceId);
  const threadOpen = Boolean(state.threadRootEventId);
  const roomMessages = messages.filter((message) => message.roomId === state.activeRoomId);
  const searchResults = search(state.searchQuery);

  app.innerHTML = `
    <div class="desktop">
      <header class="titlebar">
        <div class="traffic">
          <span class="dot red"></span>
          <span class="dot yellow"></span>
          <span class="dot green"></span>
          <span class="history">
            <button class="icon-button" type="button" aria-label="Back">‹</button>
            <button class="icon-button" type="button" aria-label="Forward">›</button>
          </span>
        </div>
        <label class="top-search">
          <span class="search-icon">⌕</span>
          <input data-action="search" value="${escapeAttr(state.searchQuery)}" placeholder="${escapeAttr(activeSpace?.name ?? "All rooms")} 内を検索する" />
        </label>
        <div class="top-actions">
          <button class="icon-button" type="button" aria-label="History">◷</button>
          <button class="icon-button" type="button" aria-label="Help">?</button>
        </div>
      </header>
      <div class="app-grid ${threadOpen ? "" : "thread-closed"}">
        ${renderWorkspaceRail()}
        ${renderSidebar(activeRoom)}
        ${renderMainPane(activeRoom, roomMessages, searchResults)}
        ${renderThreadPane(activeRoom)}
      </div>
    </div>
  `;

  const searchInput = app.querySelector('[data-action="search"]');
  searchInput?.focus({ preventScroll: true });
  if (searchInput) {
    searchInput.setSelectionRange(searchInput.value.length, searchInput.value.length);
  }
}

function renderWorkspaceRail() {
  return `
    <nav class="workspace-rail" aria-label="Workspaces">
      <div class="workspace-list">
        ${spaces
          .map((space) => {
            const count = unreadForSpace(space);
            return `
              <button class="workspace-button ${space.id === state.activeSpaceId ? "is-active" : ""}" type="button" data-space-id="${space.id}" ${count ? `data-count="${count}"` : ""} aria-label="${escapeAttr(space.name)}">
                ${escapeHtml(space.shortName)}
              </button>
            `;
          })
          .join("")}
      </div>
      <div class="rail-footer">
        <button class="rail-action" type="button" aria-label="Add workspace">+</button>
        <div class="user-presence" aria-label="Online"></div>
      </div>
    </nav>
  `;
}

function renderSidebar(activeRoom) {
  const spaceRooms = rooms.filter(
    (room) => !room.isDm && room.parentSpaceIds.includes(state.activeSpaceId),
  );
  const dms = rooms.filter((room) => room.isDm);

  return `
    <aside class="sidebar">
      <div class="workspace-header">
        <div class="workspace-name">${escapeHtml(activeSpaceName())}</div>
        <button class="icon-button" type="button" aria-label="Preferences">⚙</button>
        <button class="icon-button" type="button" aria-label="New message">✎</button>
      </div>
      <button class="upgrade" type="button"><span>↗</span><span>プランをアップグレード</span></button>
      <div class="sidebar-scroll">
        <button class="nav-item" type="button"><span>⌂</span><span class="nav-label">ホーム</span></button>
        <button class="nav-item" type="button"><span>◌</span><span class="nav-label">スレッド</span></button>
        <button class="nav-item" type="button"><span>◒</span><span class="nav-label">ハドルミーティング</span></button>
        <button class="nav-item" type="button"><span>▻</span><span class="nav-label">下書き & 送信済み</span></button>
        <div class="section-title"><span>チャンネル</span><span>＋</span></div>
        ${spaceRooms.map((room) => renderRoomItem(room, activeRoom)).join("")}
        <div class="section-title"><span>ダイレクトメッセージ</span><span>＋</span></div>
        ${dms.map((room) => renderRoomItem(room, activeRoom)).join("")}
        <div class="section-title"><span>App</span><span>＋</span></div>
        <button class="room-item" type="button"><span>◆</span><span class="room-name">Slackbot</span></button>
      </div>
    </aside>
  `;
}

function renderRoomItem(room, activeRoom) {
  const glyph = room.isDm ? "●" : "#";
  return `
    <button class="room-item ${room.id === activeRoom?.id ? "is-active" : ""}" type="button" data-room-id="${room.id}">
      <span>${glyph}</span>
      <span class="room-name">${escapeHtml(room.name)}</span>
      <span class="room-count">${room.unread ? room.unread : ""}</span>
    </button>
  `;
}

function renderMainPane(activeRoom, roomMessages, searchResults) {
  return `
    <main class="main-pane">
      <header class="channel-header">
        <div class="channel-title">
          <span>${activeRoom?.isDm ? "●" : "#"}</span>
          <span>${escapeHtml(activeRoom?.name ?? "No room")}</span>
        </div>
        <div class="channel-actions">
          <button class="member-pill" type="button" aria-label="Members"><span>◉</span><span>8</span></button>
          <button class="icon-button" type="button" data-toggle-thread aria-label="Toggle thread">⋮</button>
        </div>
      </header>
      <nav class="tabs" aria-label="Room tabs">
        <button class="tab is-active" type="button">メッセージ</button>
        <button class="tab" type="button">canvas を追加する</button>
        <button class="tab" type="button">その他</button>
        <button class="tab" type="button">＋</button>
      </nav>
      <section class="timeline-scroll">
        ${renderSearchResults(searchResults)}
        <div class="message-list">
          ${roomMessages.map(renderMessage).join("")}
        </div>
      </section>
      <section class="composer" aria-label="Message composer">
        <div class="composer-tools">
          <button class="icon-button" type="button" aria-label="Bold">B</button>
          <button class="icon-button" type="button" aria-label="Italic">I</button>
          <button class="icon-button" type="button" aria-label="Link">⌁</button>
          <button class="icon-button" type="button" aria-label="List">≡</button>
          <button class="icon-button" type="button" aria-label="Code">&lt;/&gt;</button>
        </div>
        <textarea data-action="composer" placeholder="${escapeAttr(activeRoom?.name ?? "room")} へのメッセージ">${escapeHtml(state.composerDraft)}</textarea>
        <div class="composer-footer">
          <div>
            <button class="icon-button" type="button" aria-label="Add">＋</button>
            <button class="icon-button" type="button" aria-label="Mention">@</button>
            <button class="icon-button" type="button" aria-label="Emoji">☻</button>
          </div>
          <button class="send-button ${state.composerDraft.trim() ? "ready" : ""}" type="button" aria-label="Send">➤</button>
        </div>
      </section>
    </main>
  `;
}

function renderSearchResults(results) {
  const hasQuery = state.searchQuery.trim().length > 0;
  const countLabel = results.length === 1 ? "1 result" : `${results.length} results`;
  return `
    <section class="search-results" ${hasQuery ? "" : "hidden"}>
      <div class="search-results-header">
        <span>${escapeHtml(countLabel)} for "${escapeHtml(state.searchQuery)}"</span>
        <button class="icon-button" type="button" data-clear-search aria-label="Clear search">×</button>
      </div>
      <div class="result-list">
        ${
          results.length
            ? results.map(renderSearchResult).join("")
            : '<div class="result-button"><span>No exact matches</span><span class="result-meta">verified</span></div>'
        }
      </div>
    </section>
  `;
}

function renderSearchResult(result) {
  const room = roomById(result.roomId);
  return `
    <button class="result-button" type="button" data-result-room-id="${result.roomId}" data-result-event-id="${result.eventId}">
      <span>${highlightExact(result.snippet, state.searchQuery)}</span>
      <span class="result-meta">${escapeHtml(room?.name ?? result.roomId)} · ${escapeHtml(result.field)}</span>
    </button>
  `;
}

function renderMessage(message) {
  const initials = avatarInitials(message.sender);
  return `
    <article class="message ${message.eventId === state.focusedEventId ? "is-focused" : ""}" data-event-id="${message.eventId}">
      <div class="avatar ${message.bot ? "bot" : ""}" aria-hidden="true">${escapeHtml(initials)}</div>
      <div class="message-main">
        <div class="message-heading">
          <span class="sender">${escapeHtml(message.sender)}</span>
          <span class="time">${escapeHtml(message.time)}</span>
        </div>
        <div class="message-body">${formatMessageBody(message.body)}</div>
        ${message.attachmentFilename ? `<div class="attachment">▣ ${highlightExact(message.attachmentFilename, state.searchQuery)}</div>` : ""}
        ${
          message.replyCount
            ? `<button class="reply-link" type="button" data-open-thread="${message.eventId}">新しい返信を確認する · ${message.replyCount}</button>`
            : ""
        }
      </div>
    </article>
  `;
}

function renderThreadPane(activeRoom) {
  if (!state.threadRootEventId) {
    return '<aside class="thread-pane"></aside>';
  }

  const root = messages.find((message) => message.eventId === state.threadRootEventId);
  const replies = threadReplies.filter(
    (reply) => reply.roomId === state.activeRoomId && reply.rootEventId === state.threadRootEventId,
  );

  return `
    <aside class="thread-pane">
      <header class="thread-header">
        <div class="thread-title">スレッド</div>
        <button class="icon-button" type="button" aria-label="More">⋮</button>
        <button class="icon-button" type="button" data-close-thread aria-label="Close thread">×</button>
      </header>
      <section class="thread-scroll">
        <div class="thread-root">
          ${root ? renderMessage(root) : ""}
        </div>
        ${replies.map(renderThreadReply).join("")}
      </section>
      <section class="thread-composer" aria-label="Thread composer">
        <textarea placeholder="${escapeAttr(activeRoom?.name ?? "thread")} に返信する"></textarea>
      </section>
    </aside>
  `;
}

function renderThreadReply(reply) {
  return `
    <article class="thread-reply">
      <div class="avatar" aria-hidden="true">${escapeHtml(avatarInitials(reply.sender))}</div>
      <div class="message-main">
        <div class="message-heading">
          <span class="sender">${escapeHtml(reply.sender)}</span>
          <span class="time">${escapeHtml(reply.time)}</span>
        </div>
        <div class="message-body">${formatMessageBody(reply.body)}</div>
      </div>
    </article>
  `;
}

function search(query) {
  const trimmed = query.trim();
  if (!trimmed) {
    return [];
  }

  return messages
    .map((message) => {
      if (message.body.includes(trimmed)) {
        return {
          roomId: message.roomId,
          eventId: message.eventId,
          snippet: message.body,
          field: "message",
          score: message.eventId === "$false-positive" ? 1000 : 900,
        };
      }
      if (message.attachmentFilename?.includes(trimmed)) {
        return {
          roomId: message.roomId,
          eventId: message.eventId,
          snippet: message.attachmentFilename,
          field: "attachment filename",
          score: 860,
        };
      }
      return null;
    })
    .filter(Boolean)
    .sort((left, right) => right.score - left.score || left.eventId.localeCompare(right.eventId));
}

function highlightExact(text, query) {
  const escaped = escapeHtml(text);
  if (!query.trim()) {
    return escaped;
  }
  const index = text.indexOf(query);
  if (index < 0) {
    return escaped;
  }
  const before = escapeHtml(text.slice(0, index));
  const match = escapeHtml(text.slice(index, index + query.length));
  const after = escapeHtml(text.slice(index + query.length));
  return `${before}<mark>${match}</mark>${after}`;
}

function formatMessageBody(body) {
  return body
    .split("\n")
    .map((line) => highlightExact(line, state.searchQuery))
    .join("<br />");
}

function unreadForSpace(space) {
  return rooms
    .filter((room) => !room.isDm && room.parentSpaceIds.includes(space.id))
    .reduce((sum, room) => sum + room.unread, 0);
}

function activeSpaceName() {
  return spaces.find((space) => space.id === state.activeSpaceId)?.name ?? "Koushi";
}

function roomById(roomId) {
  return rooms.find((room) => room.id === roomId);
}

function avatarInitials(name) {
  const ascii = name.match(/[A-Za-z]/g);
  if (ascii?.length) {
    return ascii.slice(0, 2).join("").toUpperCase();
  }
  return name.slice(0, 2);
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function escapeAttr(value) {
  return escapeHtml(value);
}

app.addEventListener("click", (event) => {
  const target = event.target.closest("button");
  if (!target) {
    return;
  }

  if (target.dataset.spaceId) {
    state.activeSpaceId = target.dataset.spaceId;
    const firstRoom = rooms.find(
      (room) => !room.isDm && room.parentSpaceIds.includes(state.activeSpaceId),
    );
    if (firstRoom) {
      state.activeRoomId = firstRoom.id;
      state.threadRootEventId = null;
      state.focusedEventId = null;
    }
    render();
    return;
  }

  if (target.dataset.roomId) {
    state.activeRoomId = target.dataset.roomId;
    state.threadRootEventId = null;
    state.focusedEventId = null;
    render();
    return;
  }

  if (target.dataset.openThread) {
    state.threadRootEventId = target.dataset.openThread;
    render();
    return;
  }

  if (target.dataset.closeThread !== undefined) {
    state.threadRootEventId = null;
    render();
    return;
  }

  if (target.dataset.toggleThread !== undefined) {
    state.threadRootEventId = state.threadRootEventId
      ? null
      : (messages.find(
          (message) => message.roomId === state.activeRoomId && message.replyCount > 0,
        )?.eventId ?? null);
    render();
    return;
  }

  if (target.dataset.clearSearch !== undefined) {
    state.searchQuery = "";
    state.focusedEventId = null;
    render();
    return;
  }

  if (target.dataset.resultRoomId) {
    state.activeRoomId = target.dataset.resultRoomId;
    state.focusedEventId = target.dataset.resultEventId;
    const room = roomById(state.activeRoomId);
    if (!room?.isDm && room?.parentSpaceIds[0]) {
      state.activeSpaceId = room.parentSpaceIds[0];
    }
    render();
  }
});

app.addEventListener("input", (event) => {
  if (event.target.matches('[data-action="search"]')) {
    state.searchQuery = event.target.value;
    render();
  }

  if (event.target.matches('[data-action="composer"]')) {
    state.composerDraft = event.target.value;
    app
      .querySelector(".send-button")
      ?.classList.toggle("ready", state.composerDraft.trim().length > 0);
  }
});

render();
