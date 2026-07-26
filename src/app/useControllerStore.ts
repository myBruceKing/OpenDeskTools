import { useEffect, useSyncExternalStore } from "react";

export interface ControllerStore<TState> {
  subscribe(listener: () => void): () => void;
  getSnapshot(): TState;
  start(): void;
  stop(): void;
}

export function useControllerStore<TState>(controller: ControllerStore<TState>) {
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
    controller.getSnapshot
  );

  useEffect(() => {
    controller.start();
    return () => controller.stop();
  }, [controller]);

  return state;
}
