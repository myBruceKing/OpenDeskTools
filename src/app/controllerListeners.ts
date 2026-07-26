export type ControllerListener = () => void;

export class ControllerListeners {
  private readonly listeners = new Set<ControllerListener>();

  subscribe = (listener: ControllerListener) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  notify() {
    for (const listener of this.listeners) {
      listener();
    }
  }
}
