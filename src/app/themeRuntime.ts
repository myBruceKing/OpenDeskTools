import { useEffect, useState } from "react";
import type {
  AnimationSpeed,
  BackgroundFit,
  ThemeAccent,
  ThemeBackgroundAsset,
  ThemeSnapshot
} from "./themeModel";
import { themeClient } from "./themeClient";

export type ResolvedTheme = "light" | "dark";
export type EffectiveAnimationSpeed = AnimationSpeed | "reduced";

export type ThemeRootPresentation = {
  resolvedTheme: ResolvedTheme;
  accent: ThemeAccent;
  accentText: "#171717" | "#ffffff";
  reduceTransparency: boolean;
  animationSpeed: EffectiveAnimationSpeed;
  reducedMotion: boolean;
  background: ThemeBackgroundAsset | null;
  backgroundFit: BackgroundFit;
  backgroundDim: number;
  backgroundBlur: number;
  panelOpacity: number;
  backgroundUrl: string | null;
};

export type ThemeBackgroundImageState = {
  status: "idle" | "loading" | "ready" | "error";
  url: string | null;
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

function srgbChannelLuminance(channel: number) {
  const normalized = channel / 255;
  return normalized <= 0.04045
    ? normalized / 12.92
    : ((normalized + 0.055) / 1.055) ** 2.4;
}

function colorLuminance(color: string) {
  const channels = [
    Number.parseInt(color.slice(1, 3), 16),
    Number.parseInt(color.slice(3, 5), 16),
    Number.parseInt(color.slice(5, 7), 16)
  ];
  return (
    0.2126 * srgbChannelLuminance(channels[0])
    + 0.7152 * srgbChannelLuminance(channels[1])
    + 0.0722 * srgbChannelLuminance(channels[2])
  );
}

export function readableTextColor(accent: string): "#171717" | "#ffffff" {
  if (!/^#[0-9a-f]{6}$/i.test(accent)) {
    return "#ffffff";
  }
  const accentLuminance = colorLuminance(accent);
  const darkLuminance = colorLuminance("#171717");
  const whiteContrast = 1.05 / (accentLuminance + 0.05);
  const darkContrast = (accentLuminance + 0.05) / (darkLuminance + 0.05);
  return darkContrast > whiteContrast ? "#171717" : "#ffffff";
}

export function createThemeRootPresentation(
  snapshot: ThemeSnapshot | null,
  systemDark: boolean,
  systemReducedMotion: boolean,
  backgroundUrl: string | null = null
): ThemeRootPresentation {
  return {
    resolvedTheme: resolveTheme(snapshot, systemDark),
    accent: snapshot?.accent ?? "#216bd9",
    accentText: readableTextColor(snapshot?.accent ?? "#216bd9"),
    reduceTransparency: snapshot?.reduceTransparency ?? false,
    animationSpeed: systemReducedMotion ? "reduced" : snapshot?.animationSpeed ?? "normal",
    reducedMotion: systemReducedMotion,
    background: snapshot?.background ?? null,
    backgroundFit: snapshot?.backgroundFit ?? "cover",
    backgroundDim: snapshot?.backgroundDim ?? 24,
    backgroundBlur: snapshot?.backgroundBlur ?? 6,
    panelOpacity: snapshot?.panelOpacity ?? 86,
    backgroundUrl
  };
}

export function useThemeBackgroundImage(
  assetId: string | null,
  loadImage: () => Promise<Blob> = themeClient.getBackgroundImage
): ThemeBackgroundImageState {
  const [state, setState] = useState<ThemeBackgroundImageState>({
    status: "idle",
    url: null
  });

  useEffect(() => {
    if (assetId === null) {
      setState({ status: "idle", url: null });
      return undefined;
    }
    let active = true;
    let objectUrl: string | null = null;
    setState({ status: "loading", url: null });
    void loadImage()
      .then((blob) => {
        if (!active) {
          return;
        }
        objectUrl = URL.createObjectURL(blob);
        setState({ status: "ready", url: objectUrl });
      })
      .catch((error: unknown) => {
        console.error("Unable to load the active theme background", error);
        if (active) {
          setState({ status: "error", url: null });
        }
      });

    return () => {
      active = false;
      if (objectUrl !== null) {
        URL.revokeObjectURL(objectUrl);
      }
    };
  }, [assetId, loadImage]);

  return state;
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
  const styleProperties = {
    "--accent-primary": presentation.accent,
    "--text-on-accent": presentation.accentText
  };
  const previousProperties = Object.fromEntries(
    Object.keys(styleProperties).map((name) => [name, target.style.getPropertyValue(name)])
  );

  for (const [name, value] of Object.entries(attributes)) {
    target.setAttribute(name, value);
  }
  for (const [name, value] of Object.entries(styleProperties)) {
    target.style.setProperty(name, value);
  }

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

    for (const [name, appliedValue] of Object.entries(styleProperties)) {
      if (target.style.getPropertyValue(name) !== appliedValue) {
        continue;
      }
      const previousValue = previousProperties[name];
      if (previousValue.length === 0) {
        target.style.removeProperty(name);
      } else {
        target.style.setProperty(name, previousValue);
      }
    }
  };
}

export function useDocumentTheme(presentation: ThemeRootPresentation) {
  useEffect(
    () => applyThemeRootPresentation(document.documentElement, presentation),
    [
      presentation.accent,
      presentation.accentText,
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
