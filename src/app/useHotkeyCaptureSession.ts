import { useCallback, useLayoutEffect, useRef, useState } from "react";
import type {
  HotkeyCaptureClient,
  HotkeyCaptureTokenEvent
} from "./hotkeyCaptureClient";

export type HotkeyCaptureSessionStatus =
  | "idle"
  | "starting"
  | "active"
  | "stopping"
  | "fallback";

type UseHotkeyCaptureSessionOptions = {
  client: HotkeyCaptureClient;
  onToken: (token: string) => void;
};

type StartOperation = {
  epoch: number;
  promise: Promise<string | null>;
};

type FailedStop = {
  epoch: number;
  sessionId: string;
};

const FALLBACK_MESSAGE = "系统组合捕获不可用；普通按键仍可录入。";
const STOP_FAILURE_MESSAGE = "系统组合捕获未停止，请重试。";
const STOP_RETRY_DELAYS = [0, 40, 120] as const;

export function useHotkeyCaptureSession({ client, onToken }: UseHotkeyCaptureSessionOptions) {
  const [status, setStatus] = useState<HotkeyCaptureSessionStatus>("idle");
  const [message, setMessage] = useState<string | null>(null);
  const mountedRef = useRef(true);
  const desiredActiveRef = useRef(false);
  const epochRef = useRef(0);
  const sessionIdRef = useRef<string | null>(null);
  const startOperationRef = useRef<StartOperation | null>(null);
  const stopOperationsRef = useRef(new Map<number, Promise<boolean>>());
  const failedStopRef = useRef<FailedStop | null>(null);
  const listenerReadyRef = useRef<Promise<boolean> | null>(null);
  const onTokenRef = useRef(onToken);
  onTokenRef.current = onToken;

  const stopNativeSession = useCallback(async (sessionId: string) => {
    for (const delay of STOP_RETRY_DELAYS) {
      if (delay > 0) {
        await new Promise((resolve) => window.setTimeout(resolve, delay));
      }
      try {
        await client.stop(sessionId);
        return true;
      } catch {
        // Retry this exact session a bounded number of times.
      }
    }
    return false;
  }, [client]);

  const startSession = useCallback(() => {
    const currentStart = startOperationRef.current;
    if (
      desiredActiveRef.current &&
      (sessionIdRef.current !== null || currentStart?.epoch === epochRef.current)
    ) {
      return;
    }

    const epoch = ++epochRef.current;
    desiredActiveRef.current = true;
    sessionIdRef.current = null;
    failedStopRef.current = null;
    if (mountedRef.current) {
      setStatus("starting");
      setMessage(null);
    }

    let operation!: StartOperation;
    const promise = (async () => {
      try {
        const listenerReady = await (listenerReadyRef.current ?? Promise.resolve(false));
        if (!listenerReady) {
          throw new Error("hotkey capture listener unavailable");
        }
        if (epoch !== epochRef.current || !desiredActiveRef.current) {
          return null;
        }

        const { sessionId } = await client.start();
        if (
          mountedRef.current &&
          epoch === epochRef.current &&
          desiredActiveRef.current
        ) {
          sessionIdRef.current = sessionId;
          setStatus("active");
        }
        return sessionId;
      } catch {
        if (
          mountedRef.current &&
          epoch === epochRef.current &&
          desiredActiveRef.current
        ) {
          setStatus("fallback");
          setMessage(FALLBACK_MESSAGE);
        }
        return null;
      } finally {
        if (startOperationRef.current === operation) {
          startOperationRef.current = null;
        }
      }
    })();
    operation = { epoch, promise };
    startOperationRef.current = operation;
  }, [client]);

  const stopSession = useCallback((): Promise<boolean> => {
    const epoch = epochRef.current;
    desiredActiveRef.current = false;

    const existingStop = stopOperationsRef.current.get(epoch);
    if (existingStop) {
      return existingStop;
    }

    const activeSessionId = sessionIdRef.current;
    const failedSessionId = failedStopRef.current?.epoch === epoch
      ? failedStopRef.current.sessionId
      : null;
    const startPromise = startOperationRef.current?.epoch === epoch
      ? startOperationRef.current.promise
      : null;

    if (activeSessionId === null && failedSessionId === null && startPromise === null) {
      if (mountedRef.current) {
        setStatus("idle");
        setMessage(null);
      }
      return Promise.resolve(true);
    }

    if (mountedRef.current) {
      setStatus("stopping");
      setMessage(null);
    }

    let stopPromise!: Promise<boolean>;
    stopPromise = (async () => {
      const sessionId = activeSessionId ?? failedSessionId ?? await startPromise;
      if (sessionId === null) {
        if (mountedRef.current && epoch === epochRef.current && !desiredActiveRef.current) {
          setStatus("idle");
          setMessage(null);
        }
        return true;
      }

      const stopped = await stopNativeSession(sessionId);
      if (stopped) {
        if (sessionIdRef.current === sessionId) {
          sessionIdRef.current = null;
        }
        if (failedStopRef.current?.sessionId === sessionId) {
          failedStopRef.current = null;
        }
        if (mountedRef.current && epoch === epochRef.current && !desiredActiveRef.current) {
          setStatus("idle");
          setMessage(null);
        }
        return true;
      }

      if (epoch === epochRef.current && !desiredActiveRef.current) {
        failedStopRef.current = { epoch, sessionId };
        if (mountedRef.current) {
          setStatus("fallback");
          setMessage(STOP_FAILURE_MESSAGE);
        }
      }
      return false;
    })().finally(() => {
      if (stopOperationsRef.current.get(epoch) === stopPromise) {
        stopOperationsRef.current.delete(epoch);
      }
    });
    stopOperationsRef.current.set(epoch, stopPromise);
    return stopPromise;
  }, [stopNativeSession]);

  useLayoutEffect(() => {
    mountedRef.current = true;
    let disposed = false;
    let unsubscribe: (() => void) | null = null;
    listenerReadyRef.current = client.subscribe((event: HotkeyCaptureTokenEvent) => {
      if (
        desiredActiveRef.current &&
        sessionIdRef.current !== null &&
        event.sessionId === sessionIdRef.current
      ) {
        onTokenRef.current(event.token);
      }
    })
      .then((stopListening) => {
        if (disposed) {
          stopListening();
          return false;
        }
        unsubscribe = stopListening;
        return true;
      })
      .catch(() => {
        if (mountedRef.current) {
          setStatus("fallback");
          setMessage(FALLBACK_MESSAGE);
        }
        return false;
      });

    return () => {
      disposed = true;
      mountedRef.current = false;
      listenerReadyRef.current = null;
      unsubscribe?.();
      void stopSession();
    };
  }, [client, stopSession]);

  return {
    status,
    message,
    start: startSession,
    stop: stopSession
  };
}
