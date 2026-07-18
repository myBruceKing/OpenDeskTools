import { createPortal } from "react-dom";
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties
} from "react";
import styles from "./primitives.module.css";

type HintTooltipProps = {
  content: string;
  label?: string;
  className?: string;
  symbol?: "!" | "i";
  interactive?: boolean;
};

type TooltipPosition = {
  left: number;
  top: number;
  placement: "top" | "bottom";
  arrowLeft: number;
  theme?: string;
};

const TOOLTIP_OFFSET = 8;
const EDGE_PADDING = 8;

function clamp(value: number, minimum: number, maximum: number) {
  return Math.min(Math.max(value, minimum), Math.max(minimum, maximum));
}

export function HintTooltip({
  content,
  label = "查看提示",
  className = "",
  symbol = "!",
  interactive = false
}: HintTooltipProps) {
  const tooltipId = useId();
  const anchorRef = useRef<HTMLSpanElement>(null);
  const bubbleContainerRef = useRef<HTMLSpanElement>(null);
  const bubbleRef = useRef<HTMLSpanElement>(null);
  const hideTimerRef = useRef<number | null>(null);
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<TooltipPosition | null>(null);

  const updatePosition = useCallback(() => {
    const anchor = anchorRef.current;
    const bubble = bubbleContainerRef.current;
    if (!anchor || !bubble) {
      return;
    }

    const rect = anchor.getBoundingClientRect();
    const bubbleWidth = bubble.offsetWidth;
    const bubbleHeight = bubble.offsetHeight;
    const spaceAbove = rect.top - EDGE_PADDING - TOOLTIP_OFFSET;
    const spaceBelow = window.innerHeight - rect.bottom - EDGE_PADDING - TOOLTIP_OFFSET;
    const placement = spaceAbove >= bubbleHeight || spaceAbove >= spaceBelow ? "top" : "bottom";
    const left = clamp(
      rect.right - bubbleWidth,
      EDGE_PADDING,
      window.innerWidth - EDGE_PADDING - bubbleWidth
    );
    const top = placement === "top"
      ? clamp(rect.top - TOOLTIP_OFFSET - bubbleHeight, EDGE_PADDING, window.innerHeight - EDGE_PADDING - bubbleHeight)
      : clamp(rect.bottom + TOOLTIP_OFFSET, EDGE_PADDING, window.innerHeight - EDGE_PADDING - bubbleHeight);
    const anchorCenter = rect.left + rect.width / 2;
    setPosition({
      left,
      top,
      placement,
      arrowLeft: clamp(anchorCenter - left - 5, 6, bubbleWidth - 16),
      theme: anchor.closest<HTMLElement>("[data-theme]")?.dataset.theme
    });
  }, []);

  const cancelScheduledHide = () => {
    if (hideTimerRef.current !== null) {
      window.clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }
  };
  const showTooltip = () => {
    cancelScheduledHide();
    setOpen(true);
  };
  const hideTooltip = () => {
    cancelScheduledHide();
    setOpen(false);
    setPosition(null);
  };
  const scheduleHideTooltip = () => {
    cancelScheduledHide();
    hideTimerRef.current = window.setTimeout(() => {
      const activeElement = document.activeElement;
      if (activeElement === anchorRef.current || activeElement === bubbleRef.current) {
        return;
      }
      hideTooltip();
    }, 100);
  };

  useLayoutEffect(() => {
    if (open) {
      updatePosition();
    }
  }, [content, open, updatePosition]);

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);

    return () => {
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
    };
  }, [open, updatePosition]);

  useEffect(() => () => cancelScheduledHide(), []);

  const bubbleStyle = position
    ? ({
        left: position.left,
        top: position.top,
        "--tooltip-arrow-left": `${position.arrowLeft}px`
      } as CSSProperties)
    : ({ left: 0, top: 0, visibility: "hidden" } as CSSProperties);

  return (
    <>
      <span
        ref={anchorRef}
        className={[styles.hintTooltip, className].filter(Boolean).join(" ")}
        tabIndex={0}
        role={interactive ? "button" : "img"}
        aria-label={label}
        aria-describedby={!interactive && open ? tooltipId : undefined}
        aria-haspopup={interactive ? "dialog" : undefined}
        aria-expanded={interactive ? open : undefined}
        aria-controls={interactive && open ? tooltipId : undefined}
        onBlur={(event) => {
          if (event.relatedTarget !== bubbleRef.current) {
            scheduleHideTooltip();
          }
        }}
        onFocus={showTooltip}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            hideTooltip();
          } else if (
            interactive
            && (event.key === "Enter" || event.key === " " || event.key === "ArrowDown")
            && open
          ) {
            event.preventDefault();
            bubbleRef.current?.focus();
          }
        }}
        onMouseEnter={showTooltip}
        onMouseLeave={scheduleHideTooltip}
      >
        <span className={styles.hintIcon} aria-hidden="true">
          {symbol}
        </span>
      </span>
      {open
        ? createPortal(
            <span
              ref={bubbleContainerRef}
              className={[
                styles.hintBubble,
                interactive ? styles.hintBubbleInteractive : "",
                position?.placement === "top" ? styles.hintBubbleTop : styles.hintBubbleBottom
              ]
                .filter(Boolean)
                .join(" ")}
              data-tooltip-container="true"
              data-theme={position?.theme}
              style={bubbleStyle}
              onMouseEnter={interactive ? showTooltip : undefined}
              onMouseLeave={interactive ? scheduleHideTooltip : undefined}
            >
              <span
                ref={bubbleRef}
                className={[
                  styles.hintBubbleContent,
                  interactive ? styles.hintBubbleContentInteractive : ""
                ]
                  .filter(Boolean)
                  .join(" ")}
                id={tooltipId}
                role={interactive ? "dialog" : "tooltip"}
                tabIndex={interactive ? 0 : undefined}
                aria-label={interactive ? `${label}详情` : undefined}
                aria-modal={interactive ? false : undefined}
                onBlur={(event) => {
                  if (event.relatedTarget !== anchorRef.current) {
                    scheduleHideTooltip();
                  }
                }}
                onFocus={interactive ? showTooltip : undefined}
                onKeyDown={(event) => {
                  if (interactive && event.key === "Escape") {
                    event.preventDefault();
                    anchorRef.current?.focus();
                    hideTooltip();
                  }
                }}
              >
                {content}
              </span>
            </span>,
            document.body
          )
        : null}
    </>
  );
}
