import { useEffect } from "react";

function focusAppSearch() {
  const activeElement = document.activeElement;
  const searchInput = document.querySelector<HTMLInputElement>(
    'input[data-app-search="true"]:not(:disabled)'
  );

  if (!searchInput || activeElement === searchInput) {
    return;
  }

  searchInput.focus();
  searchInput.select();
}

export function useDesktopWebViewGuards() {
  useEffect(() => {
    const preventBrowserContextMenu = (event: MouseEvent) => {
      event.preventDefault();
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLocaleLowerCase() === "f") {
        event.preventDefault();
        event.stopPropagation();
        focusAppSearch();
      }
    };

    window.addEventListener("contextmenu", preventBrowserContextMenu);
    window.addEventListener("keydown", handleKeyDown, { capture: true });

    return () => {
      window.removeEventListener("contextmenu", preventBrowserContextMenu);
      window.removeEventListener("keydown", handleKeyDown, { capture: true });
    };
  }, []);
}
