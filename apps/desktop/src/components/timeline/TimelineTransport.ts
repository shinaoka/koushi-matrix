import type {
  CoreEventPayload,
  TimelineKey,
  RequestId,
  TimelineGapId
} from "../../domain/coreEvents";
import type { ComposerDocument, TimelineScrollAnchor } from "../../domain/types";

// ---------------------------------------------------------------------------
// Transport interface (Tauri IPC, browser fake, or test mock)
// ---------------------------------------------------------------------------

export interface TimelineTransport {
  /** Subscribe to `koushi-desktop://event`; returns an unsubscribe fn. */
  listenCoreEvents(listener: (payload: CoreEventPayload) => void): () => void;
  /** Re/subscribe this key after the listener is active so InitialItems cannot be missed. */
  ensureSubscribed?(timelineKey: TimelineKey): Promise<void>;
  /** Confirm that one Room InitialItems projection committed through layout. */
  acknowledgeProjection?(
    projectionRequestId: RequestId,
    timelineKey: TimelineKey,
    generation: number,
    itemCount: number,
    targetPresent: boolean
  ): Promise<void>;
  /** Confirm that one repair-produced Room batch committed through layout. */
  acknowledgeRenderedBatch?(
    timelineKey: TimelineKey,
    actorGeneration: number,
    timelineGeneration: number,
    repairGeneration: number,
    batchId: number
  ): Promise<void>;
  /** Invoke a backward-pagination command for this timeline key. */
  paginateBackwards(timelineKey: TimelineKey): Promise<void>;
  repairTimeline?(roomId: string): Promise<void>;
  /** Send a reaction command for a timeline event. */
  sendReaction(roomId: string, eventId: string, reactionKey: string): Promise<void>;
  /** Retry a failed outbound send queue item. */
  retrySend(roomId: string, transactionId: string): Promise<void>;
  /** Cancel/delete an outbound send queue item. */
  cancelSend(roomId: string, transactionId: string): Promise<void>;
  /** Redact a reaction event. */
  redactReaction(
    roomId: string,
    eventId: string,
    reactionKey: string,
    reactionEventId: string
  ): Promise<void>;
  /** Send a read receipt for a room event. */
  sendReadReceipt(roomId: string, eventId: string, threadRootEventId?: string | null): Promise<void>;
  /** Advance the fully-read marker for a room event. */
  setFullyRead(roomId: string, eventId: string): Promise<void>;
  /** Set typing state for a room. */
  setTyping(roomId: string, isTyping: boolean): Promise<void>;
  /** Edit a timeline event's message body. */
  editMessage(
    roomId: string,
    eventId: string,
    document: ComposerDocument
  ): Promise<void>;
  /** Redact a timeline event. */
  redactMessage(roomId: string, eventId: string): Promise<void>;
  /** Pin a timeline event in the room. */
  pinEvent(roomId: string, eventId: string): Promise<void>;
  /** Unpin a timeline event in the room. */
  unpinEvent(roomId: string, eventId: string): Promise<void>;
  /** Download an event-backed media attachment. */
  downloadMedia(roomId: string, eventId: string): Promise<void>;
  /** Save an already downloaded media file through the host desktop shell. */
  saveMediaFile?(sourceUrl: string, filename: string): Promise<void>;
  /** Download a Matrix avatar thumbnail for a visible sender avatar MXC. */
  downloadAvatarThumbnail?(mxcUri: string): Promise<void>;
  /** Request a Rust-owned safe source DTO for an event-backed item. */
  loadMessageSource(roomId: string, eventId: string): Promise<void>;
  /** Request missing room keys for an undecryptable event and retry decryption. */
  requestRoomKey(
    roomId: string,
    eventId: string,
    origin: "user" | "automatic",
    timelineKey?: TimelineKey
  ): Promise<void>;
  /** Forward an event-backed message through Rust-owned source projection. */
  forwardMessage(
    roomId: string,
    sourceEventId: string,
    destinationRoomId: string
  ): Promise<void>;
  /** Request Rust-owned link-preview metadata for a timeline event. */
  loadLinkPreviews(roomId: string, eventId: string): Promise<void>;
  /** Hide the link previews for a timeline event. */
  hideLinkPreview(roomId: string, eventId: string): Promise<void>;
  /** Report viewport facts; Rust owns marker/count semantics. */
  observeViewport?(
    roomId: string,
    firstVisibleEventId: string | null,
    lastVisibleEventId: string | null,
    visibleGapIds: TimelineGapId[],
    atBottom: boolean,
    threadRootEventId: string | null
  ): Promise<void>;
  /** Persist the current room-local read/scroll anchor. */
  updateScrollAnchor?(roomId: string, anchor: TimelineScrollAnchor): Promise<void>;
  /** Resolve a timestamp through Rust and open focused context. */
  openAtTimestamp?(roomId: string, timestampMs: number): Promise<void>;
}
