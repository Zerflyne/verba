/** La barra di stato: un pallino e una riga.
 *
 *  Gli errori vivono qui, non in finestre modali. Una finestra modale
 *  interrompe quello che si sta facendo; una riga in fondo dice cosa e'
 *  successo e lascia continuare. */

export type Tono = "riposo" | "lavoro" | "ok" | "errore";

export function BarraDiStato({ tono, testo }: { tono: Tono; testo: string }) {
  const classe = tono === "riposo" ? "" : tono === "lavoro" ? "lavoro" : tono;
  return (
    <footer className={`barra-stato${tono === "errore" ? " errore" : ""}`}>
      <span className={`pallino ${classe}`} />
      <span>{testo}</span>
    </footer>
  );
}
