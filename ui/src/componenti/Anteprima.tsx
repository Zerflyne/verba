/** L'anteprima: il fotogramma al tempo `t`, disegnato dal motore.
 *
 *  Il canvas ha la risoluzione vera del video e viene scalato dal CSS. I byte
 *  arrivano gia' composti da `verba-core` — **lo stesso codice dell'export** —
 *  e qui si mettono su un canvas e basta.
 *
 *  Con un file di solo audio non c'e' un filmato sotto e il fotogramma e'
 *  trasparente dove non ci sono sottotitoli: a schermo lo si guarda su nero,
 *  che e' come lo si guarderebbe in un montaggio. La trasparenza resta intatta
 *  nel file esportato — e' il fondo dell'anteprima a essere nero, non i
 *  pixel.
 *
 *  Durante il trascinamento del cursore le richieste si limitano a una per
 *  fotogramma di schermo, e quella in volo non si accavalla con la
 *  successiva: e' l'unica ottimizzazione ammessa, perche' non cambia cosa si
 *  vede, solo quante volte lo si chiede. */

import { useEffect, useRef, useState } from "react";
import { fotogramma } from "../ponte";

interface Props {
  tempo: number;
  larghezza: number;
  altezza: number;
  /** Rettangolo dell'area sicura e linea di base dei sottotitoli. */
  guide?: boolean;
  /** La frazione di altezza su cui sta la linea di base. */
  baseline?: number;
  margine?: number;
  /** Il file di partenza e' solo audio: non c'e' un filmato sotto. */
  soloAudio?: boolean;
}

export function Anteprima({
  tempo,
  larghezza,
  altezza,
  guide = false,
  baseline = 0.82,
  margine = 0.05,
  soloAudio = false,
}: Props) {
  const tela = useRef<HTMLCanvasElement>(null);
  const inVolo = useRef(false);
  const atteso = useRef<number | null>(null);
  const richiesta = useRef(0);
  const [errore, setErrore] = useState<string | null>(null);

  useEffect(() => {
    // Ogni cambio di tempo e' una richiesta numerata: quando ne torna una
    // vecchia la si butta, cosi' trascinando il cursore non si vede un
    // fotogramma tornare indietro. La coda invece si svuota sempre — l'ultimo
    // tempo chiesto e' quello che deve restare a schermo.
    const mia = ++richiesta.current;

    const disegna = async (t: number, id: number) => {
      if (inVolo.current) {
        atteso.current = t;
        return;
      }
      inVolo.current = true;
      try {
        const immagine = await fotogramma(t, larghezza, altezza);
        if (id === richiesta.current) {
          const c = tela.current;
          if (c) c.getContext("2d")?.putImageData(immagine, 0, 0);
          setErrore(null);
        }
      } catch (e) {
        if (id === richiesta.current) setErrore(String(e));
      } finally {
        inVolo.current = false;
        const prossimo = atteso.current;
        atteso.current = null;
        if (prossimo !== null) void disegna(prossimo, richiesta.current);
      }
    };

    void disegna(tempo, mia);
  }, [tempo, larghezza, altezza]);

  return (
    <div className={`anteprima${soloAudio ? " su-nero" : ""}`}>
      <canvas ref={tela} width={larghezza} height={altezza} />
      {guide && (
        <svg className="guide" viewBox={`0 0 ${larghezza} ${altezza}`} preserveAspectRatio="none">
          <rect
            x={larghezza * margine}
            y={altezza * margine}
            width={larghezza * (1 - margine * 2)}
            height={altezza * (1 - margine * 2)}
            fill="none"
            stroke="rgba(167,139,250,0.55)"
            strokeWidth={2}
            strokeDasharray="10 8"
          />
          <line
            x1={0}
            x2={larghezza}
            y1={altezza * baseline}
            y2={altezza * baseline}
            stroke="rgba(167,139,250,0.55)"
            strokeWidth={2}
          />
        </svg>
      )}
      {errore && <div className="bottone-guide errore">{errore}</div>}
    </div>
  );
}
