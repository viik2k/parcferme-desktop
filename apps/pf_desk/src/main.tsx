import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { Dash } from "./components/Dash";
import "./index.css";

// One bundle, two windows: the Rust shell opens the dash at `index.html#dash`
// (commands::open_dash), everything else is the tray app.
const isDash = window.location.hash === "#dash";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isDash ? <Dash /> : <App />}</React.StrictMode>,
);
