import { render } from "preact";
import { App } from "./App.tsx";
import "./app.css";

if (import.meta.env.DEV) {
  await import("preact/debug");
}

const root = document.querySelector("#app");
if (root === null) {
  throw new Error("Root element #app not found");
}

render(<App />, root);
