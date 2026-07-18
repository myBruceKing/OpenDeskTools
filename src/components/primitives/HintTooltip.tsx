import { createPortal } from "react-dom";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import styles from "./primitives.module.css";

type HintTooltipProps = {
  content: string;
  label?: string;
  className?: string;
  symbol?: "!" | "i";
};

type TooltipPosition = {
  left: number;
  top: number;
  placement: "top" | "bottom";
  theme?: string;
};

const TOOLTIP_OFFSET = 8;
const EDGE_PADDING = 8;

export function HintTooltip({ content, label = "查看提示", className = "", symbol = "!" }: HintTooltipProps) {
  const tooltipId = useId();
  const anchorRef = useRef<HTMLSpanElement>(null);
  const [position, setPosition] = useState<TooltipPosition | null>(null);
  const visible = position !== null;

  const updatePosition = useCallback(() => {
    const anchor = anchorRef.current;
    if (!anchor) {
      return;
    }

    const rect = anchor.getBoundingClientRect();
    const placement = rect.top > 96 ? "top" : "bottom";
    setPosition({
      left: Math.min(window.innerWidth - EDGE_PADDING, Math.max(EDGE_PADDING, rect.right)),
      top: placement === "top" ? rect.top - TOOLTIP_OFFSET : rect.bottom + TOOLTIP_OFFSET,
      placement,
      theme: anchor.closest<HTMLElement>("[data-theme]")?.dataset.theme
    });
  }, []);

  const showTooltip = () => updatePosition();
  const hideTooltip = () => setPosition(null);

  useEffect(() => {
    if (!visible) {
      return undefined;
    }

    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);

    return () => {
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
    };
  }, [updatePosition, visible]);

  return (
    <>
      <span
        ref={anchorRef}
        className={[styles.hintTooltip, className].filter(Boolean).join(" ")}
        tabIndex={0}
        role="img"
        aria-label={label}
        aria-describedby={visible ? tooltipId : undefined}
        onBlur={hideTooltip}
        onFocus={showTooltip}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            hideTooltip();
          }
        }}
        onMouseEnter={showTooltip}
        onMouseLeave={hideTooltip}
      >
        <span className={styles.hintIcon} aria-hidden="true">
          {symbol}
        </span>
      </span>
      {position
        ? createPortal(
            <span
              className={[
                styles.hintBubble,
                position.placement === "top" ? styles.hintBubbleTop : styles.hintBubbleBottom
              ]
                .filter(Boolean)
                .join(" ")}
              id={tooltipId}
              role="tooltip"
              data-theme={position.theme}
              style={{ left: position.left, top: position.top }}
            >
              {content}
            </span>,
            document.body
          )
        : null}
    </>
  );
}
