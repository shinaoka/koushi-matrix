import {
  COMPOSER_DRAFT_REVISION_ZERO,
  compareComposerDraftRevisions,
  nextComposerDraftRevision
} from "./composerDraftRevision";
import type {
  ComposerDraftAccountOwner,
  ComposerDraftRevision,
  ComposerTarget
} from "./types";

const MAIN_TOMBSTONE_LIMIT = 128;
const THREAD_TOMBSTONE_LIMIT = 256;

export interface ComposerDraftScope {
  account: ComposerDraftAccountOwner;
  target: ComposerTarget;
}

export interface ComposerDraftLeaseSnapshot {
  rendererGeneration: string;
  leaseId: string;
  revision: ComposerDraftRevision;
  lastAcceptedClearRevision: ComposerDraftRevision;
  hasAuthoritativeContent: boolean;
}

export interface ComposerDraftLifecycleBackend {
  begin(): Promise<string>;
  acquire(
    scope: ComposerDraftScope,
    rendererGeneration: string
  ): Promise<ComposerDraftLeaseSnapshot>;
  release(lease: ComposerDraftLeaseSnapshot): Promise<void>;
}

export interface ComposerDraftOperationCapture {
  readonly scope: ComposerDraftScope;
  readonly rendererGeneration: string;
  readonly operationId: number;
}

export interface ComposerDraftLifecycleCounts {
  quiescentMain: number;
  quiescentThread: number;
  protected: number;
}

export interface ComposerDraftActiveOverlay {
  value: string;
  revision: ComposerDraftRevision | null;
}

export interface ComposerDraftLifecycleRegistry {
  activate(scope: ComposerDraftScope): Promise<ComposerDraftLeaseSnapshot>;
  observe(
    scope: ComposerDraftScope,
    revision: ComposerDraftRevision,
    lastAcceptedClearRevision: ComposerDraftRevision,
    hasAuthoritativeContent: boolean
  ): boolean;
  nextDraft(scope: ComposerDraftScope): ComposerDraftRevision;
  reserveAcceptedRevision(
    capture: ComposerDraftOperationCapture,
    submittedRevision: ComposerDraftRevision
  ): ComposerDraftRevision;
  beginOperation(scope: ComposerDraftScope): ComposerDraftOperationCapture;
  settleOperation(capture: ComposerDraftOperationCapture): boolean;
  settleOperationCompletion(
    capture: ComposerDraftOperationCapture,
    leaseId: string,
    capturedRevision: ComposerDraftRevision
  ): boolean;
  setDebounce(scope: ComposerDraftScope, handle: number): void;
  clearDebounce(scope: ComposerDraftScope): void;
  setActiveOverlay(
    scope: ComposerDraftScope,
    value: string | null,
    revision: ComposerDraftRevision | null
  ): void;
  activeOverlay(scope: ComposerDraftScope): ComposerDraftActiveOverlay | null;
  deactivate(scope: ComposerDraftScope): Promise<void>;
  revokeRendererGeneration(): void;
  counts(): ComposerDraftLifecycleCounts;
  has(scope: ComposerDraftScope): boolean;
  snapshot(scope: ComposerDraftScope): ComposerDraftLeaseSnapshot | null;
}

interface Entry {
  scope: ComposerDraftScope;
  revision: ComposerDraftRevision;
  lastAcceptedClearRevision: ComposerDraftRevision;
  hasAuthoritativeContent: boolean;
  activeOverlay: ComposerDraftActiveOverlay | null;
  active: boolean;
  debounce: number | null;
  pendingOperations: Map<
    number,
    {
      settled: Promise<void>;
      resolve: () => void;
      reservedAcceptedRevision: ComposerDraftRevision | null;
    }
  >;
  lease: ComposerDraftLeaseSnapshot | null;
  activation: Promise<ComposerDraftLeaseSnapshot> | null;
  releasePending: boolean;
  lruSequence: number | null;
}

interface AccountEntries {
  main: Map<string, Entry>;
  thread: Map<string, Map<string, Entry>>;
}

type DeviceEntries = Map<string, AccountEntries>;
type UserEntries = Map<string, DeviceEntries>;

export class ComposerDraftRendererRetiredError extends Error {
  constructor() {
    super("composer renderer generation retired");
    this.name = "ComposerDraftRendererRetiredError";
  }
}

export function createComposerDraftLifecycleRegistry(
  backend: ComposerDraftLifecycleBackend
): ComposerDraftLifecycleRegistry {
  const accounts = new Map<string, UserEntries>();
  let rendererGeneration: string | null = null;
  let rendererGenerationActivation: Promise<string> | null = null;
  let nextOperationId = 1;
  let nextLruSequence = 1;

  async function activateRendererGeneration(): Promise<string> {
    if (rendererGeneration !== null) return rendererGeneration;
    if (rendererGenerationActivation !== null) return rendererGenerationActivation;
    const activation = backend.begin();
    rendererGenerationActivation = activation;
    try {
      const generation = await activation;
      if (rendererGenerationActivation !== activation) {
        throw new ComposerDraftRendererRetiredError();
      }
      rendererGeneration = generation;
      return generation;
    } finally {
      if (rendererGenerationActivation === activation) {
        rendererGenerationActivation = null;
      }
    }
  }

  function accountEntries(
    account: ComposerDraftAccountOwner,
    create: boolean
  ): AccountEntries | undefined {
    let users = accounts.get(account.homeserver);
    if (!users && create) {
      users = new Map();
      accounts.set(account.homeserver, users);
    }
    let devices = users?.get(account.user_id);
    if (!devices && create) {
      devices = new Map();
      users?.set(account.user_id, devices);
    }
    let entries = devices?.get(account.device_id);
    if (!entries && create) {
      entries = { main: new Map(), thread: new Map() };
      devices?.set(account.device_id, entries);
    }
    return entries;
  }

  function lookup(scope: ComposerDraftScope): Entry | undefined {
    const entries = accountEntries(scope.account, false);
    if (!entries) return undefined;
    if (scope.target.kind === "main") {
      return entries.main.get(scope.target.room_id);
    }
    return entries.thread
      .get(scope.target.room_id)
      ?.get(scope.target.root_event_id);
  }

  function ensure(scope: ComposerDraftScope): Entry {
    const existing = lookup(scope);
    if (existing) return existing;
    const entries = accountEntries(scope.account, true);
    if (!entries) throw new Error("composer lifecycle account allocation failed");
    const entry: Entry = {
      scope: cloneScope(scope),
      revision: COMPOSER_DRAFT_REVISION_ZERO,
      lastAcceptedClearRevision: COMPOSER_DRAFT_REVISION_ZERO,
      hasAuthoritativeContent: false,
      activeOverlay: null,
      active: false,
      debounce: null,
      pendingOperations: new Map(),
      lease: null,
      activation: null,
      releasePending: false,
      lruSequence: null
    };
    if (scope.target.kind === "main") {
      entries.main.set(scope.target.room_id, entry);
    } else {
      let roots = entries.thread.get(scope.target.room_id);
      if (!roots) {
        roots = new Map();
        entries.thread.set(scope.target.room_id, roots);
      }
      roots.set(scope.target.root_event_id, entry);
    }
    return entry;
  }

  function remove(entry: Entry): void {
    const entries = accountEntries(entry.scope.account, false);
    if (!entries) return;
    if (entry.scope.target.kind === "main") {
      entries.main.delete(entry.scope.target.room_id);
    } else {
      const roots = entries.thread.get(entry.scope.target.room_id);
      roots?.delete(entry.scope.target.root_event_id);
      if (roots?.size === 0) entries.thread.delete(entry.scope.target.room_id);
    }
    if (entries.main.size === 0 && entries.thread.size === 0) {
      const users = accounts.get(entry.scope.account.homeserver);
      const devices = users?.get(entry.scope.account.user_id);
      devices?.delete(entry.scope.account.device_id);
      if (devices?.size === 0) users?.delete(entry.scope.account.user_id);
      if (users?.size === 0) accounts.delete(entry.scope.account.homeserver);
    }
  }

  function entries(): Entry[] {
    const result: Entry[] = [];
    for (const users of accounts.values()) {
      for (const devices of users.values()) {
        for (const account of devices.values()) {
          result.push(...account.main.values());
          for (const roots of account.thread.values()) result.push(...roots.values());
        }
      }
    }
    return result;
  }

  function isProtected(entry: Entry): boolean {
    return (
      entry.hasAuthoritativeContent ||
      (entry.activeOverlay !== null && entry.activeOverlay.value.length > 0) ||
      entry.active ||
      entry.debounce !== null ||
      entry.pendingOperations.size > 0 ||
      entry.activation !== null ||
      entry.releasePending ||
      entry.lease !== null
    );
  }

  function reconcile(entry: Entry): void {
    if (isProtected(entry)) {
      entry.lruSequence = null;
      return;
    }
    if (entry.lruSequence === null) {
      entry.lruSequence = nextLruSequence++;
    }
    evict("main", MAIN_TOMBSTONE_LIMIT);
    evict("thread", THREAD_TOMBSTONE_LIMIT);
  }

  function evict(kind: ComposerTarget["kind"], limit: number): void {
    const quiescent = entries()
      .filter((entry) => entry.scope.target.kind === kind && entry.lruSequence !== null)
      .sort((left, right) => (left.lruSequence ?? 0) - (right.lruSequence ?? 0));
    for (const victim of quiescent.slice(0, Math.max(0, quiescent.length - limit))) {
      remove(victim);
    }
  }

  async function activate(scope: ComposerDraftScope): Promise<ComposerDraftLeaseSnapshot> {
    const entry = ensure(scope);
    entry.active = true;
    entry.lruSequence = null;
    if (entry.lease) return entry.lease;
    if (entry.activation) return entry.activation;
    const activation = activateRendererGeneration().then((admittedGeneration) =>
      backend.acquire(cloneScope(scope), admittedGeneration)
    );
    entry.activation = activation;
    try {
      const lease = await activation;
      const admittedGeneration = lease.rendererGeneration;
      const current = lookup(scope);
      if (
        current !== entry ||
        rendererGeneration !== admittedGeneration ||
        lease.rendererGeneration !== admittedGeneration
      ) {
        await backend.release(lease);
        throw new ComposerDraftRendererRetiredError();
      }
      entry.revision = lease.revision;
      entry.lastAcceptedClearRevision = lease.lastAcceptedClearRevision;
      entry.hasAuthoritativeContent = lease.hasAuthoritativeContent;
      entry.lease = lease;
      return lease;
    } finally {
      if (lookup(scope) === entry) {
        entry.activation = null;
        reconcile(entry);
      }
    }
  }

  function observe(
    scope: ComposerDraftScope,
    revision: ComposerDraftRevision,
    lastAcceptedClearRevision: ComposerDraftRevision,
    hasAuthoritativeContent: boolean
  ): boolean {
    const entry = ensure(scope);
    if (entry.lease) {
      if (compareComposerDraftRevisions(revision, entry.revision) > 0) {
        entry.revision = revision;
      }
      if (
        compareComposerDraftRevisions(
          lastAcceptedClearRevision,
          entry.lastAcceptedClearRevision
        ) > 0
      ) {
        entry.lastAcceptedClearRevision = lastAcceptedClearRevision;
      }
    } else {
      entry.revision = revision;
      entry.lastAcceptedClearRevision = lastAcceptedClearRevision;
    }
    entry.hasAuthoritativeContent = hasAuthoritativeContent;
    const overlayCleared =
      entry.activeOverlay?.revision !== null &&
      entry.activeOverlay?.revision !== undefined &&
      compareComposerDraftRevisions(
        entry.lastAcceptedClearRevision,
        entry.activeOverlay.revision
      ) > 0;
    if (overlayCleared) {
      entry.activeOverlay = null;
    }
    reconcile(entry);
    return overlayCleared;
  }

  function nextDraft(scope: ComposerDraftScope): ComposerDraftRevision {
    const entry = ensure(scope);
    entry.revision = nextComposerDraftRevision(entry.revision, entry.revision);
    reconcile(entry);
    return entry.revision;
  }

  function reserveAcceptedRevision(
    capture: ComposerDraftOperationCapture,
    submittedRevision: ComposerDraftRevision
  ): ComposerDraftRevision {
    const entry = lookup(capture.scope);
    const pending = entry?.pendingOperations.get(capture.operationId);
    if (
      !entry ||
      !pending ||
      capture.rendererGeneration !== rendererGeneration ||
      !entry.lease ||
      entry.lease.rendererGeneration !== rendererGeneration
    ) {
      throw new ComposerDraftRendererRetiredError();
    }
    entry.revision = nextComposerDraftRevision(entry.revision, submittedRevision);
    pending.reservedAcceptedRevision = entry.revision;
    reconcile(entry);
    return entry.revision;
  }

  function beginOperation(scope: ComposerDraftScope): ComposerDraftOperationCapture {
    const entry = ensure(scope);
    const generation = entry.lease?.rendererGeneration;
    if (!generation || generation !== rendererGeneration) {
      throw new ComposerDraftRendererRetiredError();
    }
    const operationId = nextOperationId++;
    let resolve!: () => void;
    const settled = new Promise<void>((done) => {
      resolve = done;
    });
    entry.pendingOperations.set(operationId, {
      settled,
      resolve,
      reservedAcceptedRevision: null
    });
    entry.lruSequence = null;
    return {
      scope: cloneScope(scope),
      rendererGeneration: generation,
      operationId
    };
  }

  function settleOperation(capture: ComposerDraftOperationCapture): boolean {
    const entry = lookup(capture.scope);
    const pending = entry?.pendingOperations.get(capture.operationId);
    if (!entry || !pending) return false;
    entry.pendingOperations.delete(capture.operationId);
    pending.resolve();
    const current = capture.rendererGeneration === rendererGeneration;
    reconcile(entry);
    return current;
  }

  function settleOperationCompletion(
    capture: ComposerDraftOperationCapture,
    leaseId: string,
    capturedRevision: ComposerDraftRevision
  ): boolean {
    const entry = lookup(capture.scope);
    const pending = entry?.pendingOperations.get(capture.operationId);
    const revisionMatchesCompletion =
      entry?.revision === capturedRevision ||
      entry?.revision === pending?.reservedAcceptedRevision ||
      (entry !== undefined &&
        compareComposerDraftRevisions(
          entry.lastAcceptedClearRevision,
          capturedRevision
        ) > 0 &&
        entry.revision === entry.lastAcceptedClearRevision);
    const canApply =
      capture.rendererGeneration === rendererGeneration &&
      entry?.lease?.leaseId === leaseId &&
      revisionMatchesCompletion;
    const generationIsCurrent = settleOperation(capture);
    return generationIsCurrent && canApply;
  }

  function setDebounce(scope: ComposerDraftScope, handle: number): void {
    const entry = ensure(scope);
    entry.debounce = handle;
    entry.lruSequence = null;
  }

  function clearDebounce(scope: ComposerDraftScope): void {
    const entry = lookup(scope);
    if (!entry) return;
    entry.debounce = null;
    reconcile(entry);
  }

  function setActiveOverlay(
    scope: ComposerDraftScope,
    value: string | null,
    revision: ComposerDraftRevision | null
  ): void {
    const entry = ensure(scope);
    entry.activeOverlay = value === null ? null : { value, revision };
    reconcile(entry);
  }

  async function deactivate(scope: ComposerDraftScope): Promise<void> {
    const entry = lookup(scope);
    if (!entry) return;
    entry.active = false;
    while (entry.pendingOperations.size > 0) {
      await Promise.all(
        [...entry.pendingOperations.values()].map((operation) => operation.settled)
      );
      if (lookup(scope) !== entry) return;
    }
    const lease = entry.lease;
    entry.lease = null;
    if (!lease) {
      reconcile(entry);
      return;
    }
    entry.releasePending = true;
    try {
      await backend.release(lease);
    } finally {
      if (lookup(scope) === entry) {
        entry.releasePending = false;
        reconcile(entry);
      }
    }
  }

  function revokeRendererGeneration(): void {
    rendererGeneration = null;
    rendererGenerationActivation = null;
    for (const entry of entries()) {
      entry.active = false;
      const lease = entry.lease;
      entry.lease = null;
      if (lease) {
        entry.releasePending = true;
        void backend.release(lease).then(
          () => {
            if (lookup(entry.scope) === entry) {
              entry.releasePending = false;
              reconcile(entry);
            }
          },
          () => {
            if (lookup(entry.scope) === entry) {
              entry.releasePending = false;
              reconcile(entry);
            }
          }
        );
      }
      reconcile(entry);
    }
  }

  function counts(): ComposerDraftLifecycleCounts {
    let quiescentMain = 0;
    let quiescentThread = 0;
    let protectedCount = 0;
    for (const entry of entries()) {
      if (entry.lruSequence !== null) {
        if (entry.scope.target.kind === "main") quiescentMain += 1;
        else quiescentThread += 1;
      } else {
        protectedCount += 1;
      }
    }
    return { quiescentMain, quiescentThread, protected: protectedCount };
  }

  return {
    activate,
    observe,
    nextDraft,
    reserveAcceptedRevision,
    beginOperation,
    settleOperation,
    settleOperationCompletion,
    setDebounce,
    clearDebounce,
    setActiveOverlay,
    activeOverlay: (scope) => {
      const overlay = lookup(scope)?.activeOverlay;
      return overlay ? { ...overlay } : null;
    },
    deactivate,
    revokeRendererGeneration,
    counts,
    has: (scope) => lookup(scope) !== undefined,
    snapshot: (scope) => {
      const entry = lookup(scope);
      if (!entry) return null;
      return {
        rendererGeneration: entry.lease?.rendererGeneration ?? rendererGeneration ?? "",
        leaseId: entry.lease?.leaseId ?? "",
        revision: entry.revision,
        lastAcceptedClearRevision: entry.lastAcceptedClearRevision,
        hasAuthoritativeContent: entry.hasAuthoritativeContent
      };
    }
  };
}

function cloneScope(scope: ComposerDraftScope): ComposerDraftScope {
  return {
    account: { ...scope.account },
    target: { ...scope.target }
  };
}
