import { useId, useLayoutEffect, useRef, useState } from "react";
import { t } from "../i18n/messages";

/**
 * #1661: a Space's confirmed description, read-only, outside Space info.
 *
 * `text` is the Rust-projected topic (`SpaceSummary.topic` or
 * `SpaceChildSummary.topic`); editing stays in Space info. Long text is clamped
 * to a few lines with a toggle; whether it overflows and whether it is expanded
 * are presentation state only. Callers key the component by the Space so a
 * newly selected Space starts collapsed.
 */
export function SpaceDescriptionPreview({ text, className }: {
  text: string | null | undefined;
  className?: string;
}) {
  const textId = useId();
  const textRef = useRef<HTMLParagraphElement>(null);
  const [expanded, setExpanded] = useState(false);
  const [overflowing, setOverflowing] = useState(false);
  const hasText = Boolean(text?.trim());

  useLayoutEffect(() => {
    const element = textRef.current;
    // Measure only while clamped; expanded text keeps the last answer so the
    // toggle stays available to collapse it again.
    if (!element || expanded) return;
    const measure = () => setOverflowing(element.scrollHeight > element.clientHeight + 1);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [text, expanded]);

  if (!hasText) return null;
  return (
    <section
      className={["space-description-preview", className].filter(Boolean).join(" ")}
      aria-label={t("space.description")}
    >
      <p
        ref={textRef}
        id={textId}
        className={`space-description-text${expanded ? "" : " is-collapsed"}`}
        dir="auto"
      >
        {text}
      </p>
      {overflowing || expanded ? (
        <button
          className="space-description-toggle"
          type="button"
          aria-expanded={expanded}
          aria-controls={textId}
          onClick={() => setExpanded((value) => !value)}
        >
          {t(expanded ? "space.descriptionShowLess" : "space.descriptionShowMore")}
        </button>
      ) : null}
    </section>
  );
}
