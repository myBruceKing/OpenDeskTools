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
  subscribe: (listener: (snapshot: ThemeSnapshot) => void) => Promise<() => void>;
};

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
