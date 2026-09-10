/** Sezione 1 — Carica. Tre stati distinti nella stessa area.
 *
 *  Vuoto: l'area di trascinamento. In elaborazione: la scheda con le fasi e i
 *  loro tempi. Completato: l'anteprima con il trasporto e la striscia di
 *  parole.
 *
 *  Le fasi con i loro tempi, invece di una sola barra indefinita, hanno due
 *  vantaggi: chi aspetta capisce a che punto e', e chi guarda capisce com'e'
 *  fatta la pipeline senza che glielo si spieghi. */

import { useEffect, useState } from "react";
import type { Descrizione, NomeFase, ParolaVista, StatoModelli } from "../tipi";
import { Anteprima } from "./Anteprima";
import { Trasporto } from "./Trasporto";
import { StrisciaParole } from "./StrisciaParole";
import { durata as leggiDurata } from "./controlli";
import * as Icone from "./icone";

export interface StatoFase {
  fase: NomeFase;
  etichetta: string;
  stato: "attesa" | "in-corso" | "fatta";
  secondi: number;
}

export const ETICHETTE: Record<NomeFase, string> = {
  scaricamento: "Scaricamento dei modelli",
  preparazione: "Preparazione dell'audio",
  segmentazione: "Rilevamento del parlato",
  trascrizione: "Trascrizione",
  allineamento: "Allineamento delle parole",
  pulizia: "Pulizia dei tempi",
  impaginazione: "Impaginazione",
  codifica: "Codifica",
};

/** Le fasi che l'utente vede durante una trascrizione. */
export const FASI_TRASCRIZIONE: NomeFase[] = [
  "preparazione",
  "segmentazione",
  "trascrizione",
  "allineamento",
  "pulizia",
];

interface Props {
  file: Descrizione | null;
  elaborando: boolean;
  fasi: StatoFase[];
  frazione: number;
  parole: ParolaVista[];
  tempo: number;
  dimensioni: [number, number] | null;
  onda: number[];
  inRiproduzione: boolean;
  onScegli: () => void;
  onAnnulla: () => void;
  onTempo: (t: number) => void;
  onRiproduzione: (v: boolean) => void;
  sopra: boolean;
  /** Lo stato dei modelli: senza, qui non si trascrive niente. */
  modelli: StatoModelli | null;
  scaricando: boolean;
  frazioneScarico: number;
  onScarica: () => void;
  /** Quanti termini noti sono gia' impostati. */
  termini: number;
  onTermini: () => void;
  /** L'indirizzo della traccia da suonare, se c'e'. */
  audio: string | null;
}

const ESTENSIONI =
  "Audio: mp3, wav, m4a, flac, ogg, opus · Video: mp4, mov, mkv, webm, avi";

export function Carica(p: Props) {
  const completato = p.file && !p.elaborando && p.parole.length > 0;

  return (
    <section className="sezione">
      <header className="testata">
        <h1 className="titolo-sezione">Carica</h1>
        {p.file && <Targhetta file={p.file} />}
        {p.file && !p.elaborando && (
          <button className="pulsante a-destra" onClick={p.onScegli}>
            <Icone.File />
            Cambia file
          </button>
        )}
      </header>

      <div className="corpo">
        {!p.file &&
          (p.modelli && !p.modelli.pronto ? (
            <ModelliMancanti
              modelli={p.modelli}
              scaricando={p.scaricando}
              frazione={p.frazioneScarico}
              onScarica={p.onScarica}
            />
          ) : (
            <AreaVuota
              sopra={p.sopra}
              onScegli={p.onScegli}
              termini={p.termini}
              onTermini={p.onTermini}
            />
          ))}

        {p.file && p.elaborando && (
          <InElaborazione file={p.file} fasi={p.fasi} frazione={p.frazione} onAnnulla={p.onAnnulla} />
        )}

        {completato && p.dimensioni && (
          <Completato
            larghezza={p.dimensioni[0]}
            altezza={p.dimensioni[1]}
            tempo={p.tempo}
            durata={p.file!.durata}
            parole={p.parole}
            onda={p.onda}
            inRiproduzione={p.inRiproduzione}
            onTempo={p.onTempo}
            onRiproduzione={p.onRiproduzione}
            soloAudio={p.file!.modalita === "audio"}
            audio={p.audio}
          />
        )}

        {p.file && !p.elaborando && p.parole.length === 0 && (
          <div className="scheda-lavoro">
            <h2>{p.file.nome}</h2>
            <p className="caratteristiche">{p.file.riassunto}</p>
            <p className="nota">
              Il file e' pronto. La trascrizione parte da sola; se si e' fermata, basta ricaricare
              il file.
            </p>
          </div>
        )}
      </div>
    </section>
  );
}

function Targhetta({ file }: { file: Descrizione }) {
  return (
    <span className="targhetta">
      {file.modalita === "video" ? <Icone.Filmato /> : <Icone.Suono />}
      <span className="nome-file">{file.nome}</span>
      <span className="separatore">|</span>
      <span>
        {file.modalita === "video" ? `${file.larghezza} × ${file.altezza}` : "solo audio"}
      </span>
    </span>
  );
}

function AreaVuota({
  sopra,
  onScegli,
  termini,
  onTermini,
}: {
  sopra: boolean;
  onScegli: () => void;
  termini: number;
  onTermini: () => void;
}) {
  return (
    <div className="blocco-vuoto">
      <div className={`area-trascinamento${sopra ? " sopra" : ""}`}>
        <Icone.Carica size={34} />
        <span className="invito">Trascina qui un file audio o video</span>
        <span className="oppure">oppure</span>
        <button className="pulsante" onClick={onScegli}>
          Scegli un file
        </button>
      </div>
      <p className="formati-accettati">{ESTENSIONI}</p>

      {/* I termini vanno decisi *prima*: dopo, per applicarli, tocca
        * ritrascrivere tutto. Metterli solo in Impostazioni voleva dire che
        * se ne accorgeva chi ci era gia' passato. */}
      <div className="suggerimento">
        <span className="etichetta-consiglio">Consigliato</span>
        <div className="testo-consiglio">
          <strong>Termini noti</strong>
          <span>
            {termini > 0
              ? `${termini} fra nomi propri, sigle e parole tecniche. Vanno decisi adesso: applicarli dopo vuol dire ritrascrivere.`
              : "Nomi propri, sigle, parole tecniche: quelle che il modello sbaglierebbe. Vanno decisi adesso — applicarli dopo vuol dire ritrascrivere."}
          </span>
        </div>
        <button className="pulsante a-destra" onClick={onTermini}>
          {termini > 0 ? "Modifica" : "Aggiungili"}
        </button>
      </div>
    </div>
  );
}

/** Senza modelli non si trascrive: e' la prima cosa da dire, non l'ultima.
 *
 *  Prima questo caso finiva in una riga della barra di stato, in basso a
 *  sinistra, e chi non l'aveva letta restava a guardare un file che non
 *  partiva. Qui prende il posto dell'area di trascinamento — che senza modelli
 *  non porterebbe comunque da nessuna parte — e dice cosa manca, quanto pesa e
 *  come rimediare, con il pulsante nello stesso posto in cui si legge il
 *  problema. */
function ModelliMancanti({
  modelli,
  scaricando,
  frazione,
  onScarica,
}: {
  modelli: StatoModelli;
  scaricando: boolean;
  frazione: number;
  onScarica: () => void;
}) {
  const mancanti = modelli.modelli.filter((m) => !m.presente && m.in_uso);
  return (
    <div className="blocco-vuoto">
      <div className="scheda-avviso">
        <div className="testata-avviso">
          <Icone.Avviso size={22} />
          <h2>Mancano i modelli: senza, la trascrizione non parte</h2>
        </div>
        <p className="caratteristiche">
          Verba lavora in locale, quindi i modelli devono stare su questa macchina. Si scaricano una
          volta sola.
        </p>

        <div className="fasi">
          {mancanti.map((m) => (
            <div className="fase" key={m.id}>
              <span className="segno">·</span>
              <span>{m.nome}</span>
              <span className="tempo">
                {m.ripresa > 0 ? "ripreso a meta'" : m.leggibile}
              </span>
            </div>
          ))}
        </div>

        {scaricando ? (
          <>
            <div className="barra" style={{ marginTop: 20 }}>
              <i style={{ width: `${(frazione * 100).toFixed(1)}%` }} />
            </div>
            <p className="nota">
              Uno scaricamento interrotto riprende da dov'era: non ricomincia da capo.
            </p>
          </>
        ) : (
          <div className="bottoni-in-fila" style={{ marginTop: 20 }}>
            {modelli.da_scaricare > 0 && (
              <button className="pulsante primario" onClick={onScarica}>
                <Icone.Scarica />
                Scarica quelli che mancano ({modelli.da_scaricare_leggibile})
              </button>
            )}
          </div>
        )}

        {modelli.a_mano.map((testo, i) => (
          <p className="nota" key={i} style={{ whiteSpace: "pre-line" }}>
            {testo}
          </p>
        ))}
      </div>
    </div>
  );
}

function InElaborazione({
  file,
  fasi,
  frazione,
  onAnnulla,
}: {
  file: Descrizione;
  fasi: StatoFase[];
  frazione: number;
  onAnnulla: () => void;
}) {
  // Il tempo della fase in corso scorre.
  const [ora, setOra] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setOra((x) => x + 1), 1000);
    return () => clearInterval(id);
  }, []);

  return (
    <div className="scheda-lavoro">
      <h2>{file.nome}</h2>
      <p className="caratteristiche">{file.riassunto}</p>

      <div className="fasi">
        {fasi.map((f) => (
          <div key={f.fase} className={`fase ${f.stato === "attesa" ? "" : f.stato}`}>
            <span className="segno">
              {f.stato === "fatta" ? "✓" : f.stato === "in-corso" ? "▸" : "·"}
            </span>
            <span>{f.etichetta}</span>
            {f.stato !== "attesa" && (
              <span className="tempo" data-ora={ora}>
                {leggiDurata(f.secondi)}
              </span>
            )}
          </div>
        ))}
      </div>

      <div className="barra">
        <i style={{ width: `${(frazione * 100).toFixed(1)}%` }} />
      </div>

      <div className="bottoni-in-fila" style={{ marginTop: 24 }}>
        <button className="pulsante" onClick={onAnnulla}>
          Annulla
        </button>
      </div>
    </div>
  );
}

function Completato({
  larghezza,
  altezza,
  tempo,
  durata,
  parole,
  onda,
  inRiproduzione,
  onTempo,
  onRiproduzione,
  soloAudio,
  audio,
}: {
  larghezza: number;
  altezza: number;
  tempo: number;
  durata: number;
  parole: ParolaVista[];
  onda: number[];
  inRiproduzione: boolean;
  onTempo: (t: number) => void;
  onRiproduzione: (v: boolean) => void;
  soloAudio: boolean;
  audio: string | null;
}) {
  const [guide, setGuide] = useState(false);
  const attiva = parole.find((p) => tempo >= p.inizio && tempo < p.fine) ?? null;

  // La finestra di parole intorno alla posizione: undici entrano nella
  // larghezza senza doverla far scorrere.
  const centro = Math.max(
    0,
    Math.min(
      parole.findIndex((p) => tempo < p.fine) < 0
        ? parole.length - 1
        : parole.findIndex((p) => tempo < p.fine),
      parole.length - 1,
    ),
  );
  const da = Math.max(0, Math.min(centro - 4, Math.max(0, parole.length - 9)));
  const finestra = parole.slice(da, da + 9);

  return (
    <>
      <div style={{ position: "relative", flex: 1, minHeight: 0, display: "flex" }}>
        <Anteprima
          tempo={tempo}
          larghezza={larghezza}
          altezza={altezza}
          guide={guide}
          soloAudio={soloAudio}
        />
        <button
          className={`bottone-guide${guide ? " accese" : ""}`}
          onClick={() => setGuide(!guide)}
          title="Area sicura e linea di base"
        >
          Guide
        </button>
      </div>
      <Trasporto
        tempo={tempo}
        durata={durata}
        inRiproduzione={inRiproduzione}
        onTempo={onTempo}
        onRiproduzione={onRiproduzione}
        onda={onda}
        audio={audio}
        bloccoAttivo={
          finestra.length ? [finestra[0].inizio, finestra[finestra.length - 1].fine] : null
        }
      />
      <StrisciaParole parole={finestra} attiva={attiva?.id ?? null} onVai={onTempo} />
    </>
  );
}
