import React from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./stile.css";
import { finto } from "./ponte";

if (finto) {
  // Non deve mai passare per vero: chi apre la finestra in un browser lo
  // legge subito nella console.
  console.warn(
    "Verba: il motore non c'e'. Questa e' l'interfaccia con i dati del banco di prova; nessun file viene letto o scritto.",
  );
}

createRoot(document.getElementById("radice")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
