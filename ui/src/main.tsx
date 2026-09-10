import React from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./stile.css";
import { finto, riporta } from "./ponte";

if (finto) {
  // Non deve mai passare per vero: chi apre la finestra in un browser lo
  // legge subito nella console.
  console.warn(
    "Verba: il motore non c'e'. Questa e' l'interfaccia con i dati del banco di prova; nessun file viene letto o scritto.",
  );
}

// Tutto cio' che sfugge finisce nel log dell'applicazione. Senza, una
// eccezione qui dentro non lascia traccia da nessuna parte e chi la subisce
// puo' solo riferirne il messaggio a memoria.
window.addEventListener("error", (e) => riporta(`errore non gestito: ${e.message}`, e.error));
window.addEventListener("unhandledrejection", (e) =>
  riporta("promessa rifiutata e mai raccolta", e.reason),
);

createRoot(document.getElementById("radice")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
