import { useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { ModalDialog, NativeModal, useModalFocusTracking } from "../src/components/ModalDialog";
import { CreateEntityDialog } from "../src/components/dialogs";
import { EmojiPicker } from "../src/components/EmojiPicker";
import { ImeTextField } from "../src/components/ImeTextControl";
import { FloatingLayer, floatingPlacementStyle, useFloatingPlacement } from "../src/components/floatingLayer";
import "../src/styles.css";

function Popup({ anchor, onClose }: { anchor: React.RefObject<HTMLButtonElement | null>; onClose: () => void }) {
  const placement = useFloatingPlacement({ anchorRef: anchor, placement: "above", align: "end", inlineSize: 300, blockSize: 500 });
  return <FloatingLayer><div data-testid="floating" style={{ ...floatingPlacementStyle(placement), position: "fixed", overflow: "auto", background: "white" }}
    onKeyDown={event => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onClose(); anchor.current?.focus(); } }}>
    <button onClick={onClose}>Popup action</button>
  </div></FloatingLayer>;
}

function Harness() {
  useModalFocusTracking();
  const [open, setOpen] = useState(false);
  const [child, setChild] = useState(false);
  const [popup, setPopup] = useState(false);
  const [emoji, setEmoji] = useState(false);
  const [legacy, setLegacy] = useState(false);
  const [viewer, setViewer] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const emojiAnchor = useRef<HTMLButtonElement>(null);
  return <>
    <button onClick={() => setOpen(true)}>Open parent</button>
    <button onClick={() => setLegacy(true)}>Open legacy</button>
    {["media-viewer-backdrop", "timeline-media-viewer-overlay", "dialog-overlay upload-staging-overlay"].map(name =>
      <button key={name} onClick={() => setViewer(name)}>{name}</button>)}
    {open && <ModalDialog title="Parent" className="user-settings-modal" onClose={() => setOpen(false)}>
      <div style={{ minHeight: 0, overflow: "auto", padding: 12 }}>
        <ImeTextField aria-label="Draft" syncKey="synthetic-modal-draft" />
        <button onClick={() => setChild(true)}>Open child</button>
        <button ref={anchor} onClick={() => setPopup(true)}>Open popup</button>
        <button ref={emojiAnchor} onClick={() => setEmoji(true)}>Open modal emoji</button>
        {emoji && <EmojiPicker anchorRef={emojiAnchor} onSelect={() => setEmoji(false)} onClose={() => setEmoji(false)} />}
        {popup && <Popup anchor={anchor} onClose={() => setPopup(false)} />}
        {child && <ModalDialog title="Child" className="desktop-update-modal" dismissible={!busy} onClose={() => setChild(false)}>
          <div className="desktop-update-content">
            <ImeTextField aria-label="Child input" syncKey="synthetic-child" />
            <button onClick={() => setBusy(value => !value)}>Toggle busy</button>
            <div style={{ height: 900 }}>Synthetic long content</div>
            <button onClick={() => setChild(false)}>Child last action</button>
          </div>
        </ModalDialog>}
      </div>
    </ModalDialog>}
    {legacy && <CreateEntityDialog kind="room" isBusy={false} value="Synthetic room" onValueChange={() => undefined} onSubmit={() => undefined} onCancel={() => setLegacy(false)} />}
    {viewer && <NativeModal className={viewer} aria-label="Viewer" onDismiss={() => setViewer(null)}>
      <div className={viewer.startsWith("media") ? "media-viewer" : viewer.startsWith("timeline") ? "timeline-media-viewer" : "upload-staging-dialog"}>
        <button onClick={() => setViewer(null)}>Close viewer</button>
        <div style={{ minHeight: 0, overflow: "auto" }}><div style={{ height: 900 }}>Synthetic media</div></div>
        <button>Viewer last action</button>
      </div>
    </NativeModal>}
  </>;
}
createRoot(document.getElementById("root")!).render(<Harness />);
