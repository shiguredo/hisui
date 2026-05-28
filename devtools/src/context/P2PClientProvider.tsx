import { createContext } from "preact";
import { useContext, useEffect, useMemo } from "preact/hooks";
import type { ComponentChildren } from "preact";
import { createP2PClient, type P2PClient } from "../p2p/client.ts";

const P2PClientContext = createContext<P2PClient | null>(null);

interface P2PClientProviderProps {
  children: ComponentChildren;
}

export function P2PClientProvider({ children }: P2PClientProviderProps) {
  const client = useMemo(() => createP2PClient(), []);

  useEffect(() => {
    return () => {
      client.dispose();
    };
  }, [client]);

  return <P2PClientContext.Provider value={client}>{children}</P2PClientContext.Provider>;
}

export function useP2PClient(): P2PClient {
  const client = useContext(P2PClientContext);
  if (client === null) {
    throw new Error("useP2PClient must be used within P2PClientProvider");
  }
  return client;
}
