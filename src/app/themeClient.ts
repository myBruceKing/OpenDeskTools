import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import {
  parseThemeSnapshot,
  parseThemeUpdateResult,
  type ThemePatch,
  type ThemeSnapshot,
  type ThemeUpdateResult
} from "./themeModel";

type InvokeFunction = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
type ListenFunction = (
  event: string,
  handler: (event: { payload: unknown }) => void
) => Promise<() => void>;

export type ThemeClient = {
  get: () => Promise<ThemeSnapshot>;
  update: (expectedRevision: number, patch: ThemePatch) => Promise<ThemeUpdateResult>;
  selectBackground: (expectedRevision: number) => Promise<ThemeUpdateResult | null>;
  removeBackground: (expectedRevision: number) => Promise<ThemeUpdateResult>;
  getBackgroundImage: () => Promise<Blob>;
  subscribe: (listener: (snapshot: ThemeSnapshot) => void) => Promise<() => void>;
};

function imageBlob(value: unknown) {
  if (value instanceof ArrayBuffer) {
    return new Blob([value], { type: "image/png" });
  }
  if (value instanceof Uint8Array) {
    return new Blob([Uint8Array.from(value)], { type: "image/png" });
  }
  throw new Error("Invalid theme background image payload");
}

export function createThemeClient({
  invokeFunction = invoke as InvokeFunction,
  listenFunction = listen as ListenFunction
}: {
  invokeFunction?: InvokeFunction;
  listenFunction?: ListenFunction;
} = {}): ThemeClient {
  return {
    async get() {
      return parseThemeSnapshot(await invokeFunction("get_theme_preferences"));
    },

    async update(expectedRevision, patch) {
      return parseThemeUpdateResult(
        await invokeFunction("update_theme_preferences", {
          patch: { expectedRevision, ...patch }
        })
      );
    },

    async selectBackground(expectedRevision) {
      const value = await invokeFunction("select_theme_background", {
        input: { expectedRevision }
      });
      return value === null ? null : parseThemeUpdateResult(value);
    },

    async removeBackground(expectedRevision) {
      return parseThemeUpdateResult(
        await invokeFunction("remove_theme_background", {
          input: { expectedRevision }
        })
      );
    },

    async getBackgroundImage() {
      return imageBlob(await invokeFunction("get_theme_background_image"));
    },

    subscribe(listener) {
      return listenFunction("theme://changed", (event) => {
        try {
          listener(parseThemeSnapshot(event.payload));
        } catch (error) {
          console.error("Ignoring an invalid theme change event", error);
        }
      });
    }
  };
}

export const themeClient = createThemeClient();
