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
 *  Una richiesta alla volta, e chi arriva mentre una e' in volo prende
 *  l'unico posto in coda: durante la riproduzione i tempi arrivano decine di
 *  volte al secondo e il motore non sta dietro a tutti, ma l'ultimo chiesto
 *  finisce sempre a schermo. E' l'unica ottimizzazione ammessa, perche' non
 *  cambia cosa si vede, solo quante volte lo si chiede.
 *
 *  Cio' che torna **non si butta perche' e' vecchio**: con una sola richiesta
 *  in volo il suo risultato e' sempre il piu' recente che sia stato chiesto.
 *  Si butta solo se nel frattempo sono cambiate le dimensioni, perche' allora
 *  non entrerebbe nella tela. Confrontare invece con un numero che cresce a
 *  ogni cambio di tempo scartava tutto: durante la riproduzione il numero
 *  cambiava sempre prima che la risposta arrivasse, e l'anteprima restava
 *  ferma sul fotogramma da cui era partita. */

import { useCallback, useEffect, useRef, useState } from "react";
import { fotogramma, riporta } from "../ponte";

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
  /** Una richiesta alla volta: la successiva aspetta il suo turno. */
  const inVolo = useRef(false);
  /** L'ultimo tempo chiesto mentre una richiesta era in volo. */
  const atteso = useRef<number | null>(null);
  /** Cambia solo quando cambiano le dimensioni. */
  const generazione = useRef(0);
  const [errore, setErrore] = useState<string | null>(null);

  // Un fotogramma di 1080x1920 non entra in una tela di altre misure: quando
  // le dimensioni cambiano, cio' che e' partito prima non va piu' disegnato.
  // E' l'**unico** motivo per buttare un risultato — questo effetto sta prima
  // dell'altro perche' deve numerare la generazione nuova per primo.
  useEffect(() => {
    generazione.current++;
  }, [larghezza, altezza]);

  const disegna = useCallback(
    async (t: number) => {
      const mia = generazione.current;
      inVolo.current = true;
      try {
        const immagine = await fotogramma(t, larghezza, altezza);
        if (mia === generazione.current) {
          const c = tela.current;
          if (c) {
            const g = c.getContext("2d");
            if (!g) throw new Error(`nessun contesto 2d su una tela ${larghezza}x${altezza}`);
            g.putImageData(immagine, 0, 0);
          }
          setErrore(null);
        }
      } catch (e) {
        if (mia === generazione.current) {
          riporta(`anteprima a t=${t.toFixed(2)} (${larghezza}x${altezza})`, e);
          setErrore(String(e));
        }
      } finally {
        inVolo.current = false;
        const prossimo = atteso.current;
        atteso.current = null;
        if (prossimo !== null) void disegna(prossimo);
      }
    },
    [larghezza, altezza],
  );

  useEffect(() => {
    // La coda tiene un solo posto: durante la riproduzione arrivano decine di
    // tempi al secondo e uno solo alla volta puo' viaggiare, ma l'**ultimo**
    // chiesto deve sempre finire a schermo.
    if (inVolo.current) {
      atteso.current = tempo;
      return;
    }
    void disegna(tempo);
  }, [tempo, disegna]);

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
