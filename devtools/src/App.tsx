import { LocationProvider, Router, Route, useLocation } from "preact-iso/router";
import { P2PClientProvider } from "./context/P2PClientProvider.tsx";
import { P2PPage } from "./pages/P2PPage.tsx";
import { DebugPage } from "./pages/DebugPage.tsx";

function Nav() {
  const { path } = useLocation();

  function linkClass(targetPath: string): string {
    const active = path === targetPath;
    return active
      ? "rounded-md px-3 py-1.5 text-base font-medium bg-white/20 text-white ring-1 ring-inset ring-white/30"
      : "rounded-md px-3 py-1.5 text-base font-medium text-white/85 hover:bg-white/10 hover:text-white";
  }

  return (
    <nav class="flex items-center gap-4 border-b border-hisui-800 bg-linear-to-r from-hisui-600 to-hisui-700 px-4 py-2.5 shadow-sm">
      <h1 class="text-xl font-bold tracking-tight text-white">Hisui DevTools</h1>
      <a href="/" class={linkClass("/")}>
        P2P
      </a>
      <a href="/debug" class={linkClass("/debug")}>
        Debug
      </a>
    </nav>
  );
}

export function App() {
  return (
    <P2PClientProvider>
      <LocationProvider>
        <div class="flex h-screen flex-col bg-surface-50 text-base text-slate-900">
          <Nav />
          <Router>
            <Route path="/" component={P2PPage} />
            <Route path="/debug" component={DebugPage} />
          </Router>
        </div>
      </LocationProvider>
    </P2PClientProvider>
  );
}
