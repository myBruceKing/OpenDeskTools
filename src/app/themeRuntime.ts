import { useEffect, useState } from "react";
import type { AnimationSpeed, ThemeAccent, ThemeSnapshot } from "./themeModel";

export type ResolvedTheme = "light" | "dark";
export type EffectiveAnimationSpeed = AnimationSpeed | "reduced";

export type ThemeRootPresentation = {
  resolvedTheme: ResolvedTheme;
  accent: ThemeAccent;
  reduceTransparency: boolean;
  animationSpeed: EffectiveAnimationSpeed;
  reducedMotion: boolean;
};

type ThemeRootTarget = {
  getAttribute: (name: string) => string | null;
  setAttribute: (name: string, value: string) => void;
  removeAttribute: (name: string) => void;
  style: {
    getPropertyValue: (name: string) => string;
    setProperty: (name: string, value: string) => void;
    removeProperty: (name: string) => string;
  };
};

type MediaQueryListLike = {
  matches: boolean;
  addEventListener: (type: "change", listener: (event: { matches: boolean }) => void) => void;
  removeEventListener: (type: "change", listener: (event: { matches: boolean }) => void) => void;
};

type MatchMediaLike = (query: string) => MediaQueryListLike;

export function observeMediaQuery(
  matchMedia: MatchMediaLike,
  query: string,
  listener: (matches: boolean) => void
) {
  const mediaQuery = matchMedia(query);
  const handleChange = (event: { matches: boolean }) => listener(event.matches);
  listener(mediaQuery.matches);
  mediaQuery.addEventListener("change", handleChange);
  return () => mediaQuery.removeEventListener("change", handleChange);
}

export function resolveTheme(snapshot: ThemeSnapshot | null, systemDark: boolean): ResolvedTheme {
  if (snapshot?.mode === "dark") {
    return "dark";
  }
  if (snapshot?.mode === "system") {
    return systemDark ? "dark" : "light";
  }
  return "light";
}

export function createThemeRootPresentation(
  snapshot: ThemeSnapshot | null,
  systemDark: boolean,
  systemReducedMotion: boolean
): ThemeRootPresentation {
  return {
    resolvedTheme: resolveTheme(snapshot, systemDark),
    accent: snapshot?.accent ?? "#216bd9",
    reduceTransparency: snapshot?.reduceTransparency ?? false,
    animationSpeed: systemReducedMotion ? "reduced" : snapshot?.animationSpeed ?? "normal",
    reducedMotion: systemReducedMotion
  };
}

export function applyThemeRootPresentation(
  target: ThemeRootTarget,
  presentation: ThemeRootPresentation
) {
  const attributes = {
    "data-theme": presentation.resolvedTheme,
    "data-accent": presentation.accent,
    "data-reduce-transparency": String(presentation.reduceTransparency),
    "data-animation-speed": presentation.animationSpeed,
    "data-reduced-motion": String(presentation.reducedMotion)
  };
  const previousAttributes = Object.fromEntries(
    Object.keys(attributes).map((name) => [name, target.getAttribute(name)])
  );
  const accentProperty = "--accent-primary";
  const previousAccent = target.style.getPropertyValue(accentProperty);

  for (const [name, value] of Object.entries(attributes)) {
    target.setAttribute(name, value);
  }
  target.style.setProperty(accentProperty, presentation.accent);

  return () => {
    for (const [name, appliedValue] of Object.entries(attributes)) {
      if (target.getAttribute(name) !== appliedValue) {
        continue;
      }
      const previousValue = previousAttributes[name];
      if (previousValue === null) {
        target.removeAttribute(name);
      } else {
        target.setAttribute(name, previousValue);
      }
    }

    if (target.style.getPropertyValue(accentProperty) === presentation.accent) {
      if (previousAccent.length === 0) {
        target.style.removeProperty(accentProperty);
      } else {
        target.style.setProperty(accentProperty, previousAccent);
      }
    }
  };
}

export function useDocumentTheme(presentation: ThemeRootPresentation) {
  useEffect(
    () => applyThemeRootPresentation(document.documentElement, presentation),
    [
      presentation.accent,
      presentation.animationSpeed,
      presentation.reduceTransparency,
      presentation.reducedMotion,
      presentation.resolvedTheme
    ]
  );
}

export function useSystemThemePreferences() {
  const [systemDark, setSystemDark] = useState(false);
  const [systemReducedMotion, setSystemReducedMotion] = useState(false);

  useEffect(() => {
    const stopColorScheme = observeMediaQuery(
      window.matchMedia.bind(window),
      "(prefers-color-scheme: dark)",
      setSystemDark
    );
    const stopReducedMotion = observeMediaQuery(
      window.matchMedia.bind(window),
      "(prefers-reduced-motion: reduce)",
      setSystemReducedMotion
    );

    return () => {
      stopColorScheme();
      stopReducedMotion();
    };
  }, []);

  return { systemDark, systemReducedMotion };
}
