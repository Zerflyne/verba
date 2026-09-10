/** L'applicazione: quattro sezioni, una barra laterale, una barra di stato.
 *
 *  Non e' una procedura guidata, e' un'applicazione con tre stanze piu' le
 *  impostazioni: una volta che c'e' una trascrizione si va avanti e indietro
 *  liberamente. */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as ponte from "./ponte";
import type {
  Descrizione,
  Evento,
  Famiglia,
  FormatiDisponibili,
  Impostazioni as DatiImpostazioni,
  Info,
  NomeFase,
  ParolaVista,
  Preset,
  Riepilogo,
  Sezione,
  StatoModelli,
} from "./tipi";
import { BarraLaterale } from "./componenti/BarraLaterale";
import { BarraDiStato, type Tono } from "./componenti/BarraDiStato";
import { Carica, ETICHETTE, FASI_TRASCRIZIONE, type StatoFase } from "./componenti/Carica";
import { Modifica } from "./componenti/Modifica";
import { Esporta, type StatoExport } from "./componenti/Esporta";
import { Impostazioni } from "./componenti/Impostazioni";
import { durata as leggiDurata } from "./componenti/controlli";

const FILTRI_MEDIA = [
  { name: "Audio o video", extensions: ["mp3", "wav", "m4a", "flac", "ogg", "opus", "mp4", "mov", "mkv", "webm", "avi"] },
];

export default function App() {
  const [sezione, setSezione] = useState<Sezione>("carica");
  const [file, setFile] = useState<Descrizione | null>(null);
  const [sopra, setSopra] = useState(false);

  const [elaborando, setElaborando] = useState(false);
  const [fasi, setFasi] = useState<StatoFase[]>([]);
  const [frazione, setFrazione] = useState(0);

  const [parole, setParole] = useState<ParolaVista[]>([]);
  const [onda, setOnda] = useState<number[]>([]);
  const [dimensioni, setDimensioni] = useState<[number, number] | null>(null);
  const [riepilogo, setRiepilogo] = useState<Riepilogo | null>(null);

  const [tempo, setTempo] = useState(0);
  const [inRiproduzione, setInRiproduzione] = useState(false);

  const [preset, setPreset] = useState<Preset | null>(null);
  const [caratteri, setCaratteri] = useState<Famiglia[]>([]);
  const [avvisoCarattere, setAvvisoCarattere] = useState<string | null>(null);

  const [impostazioni, setImpostazioni] = useState<DatiImpostazioni | null>(null);
  const [impostazioniSalvate, setImpostazioniSalvate] = useState<DatiImpostazioni | null>(null);
  const [modelli, setModelli] = useState<StatoModelli | null>(null);
  const [scaricando, setScaricando] = useState(false);
  const [info, setInfo] = useState<Info | null>(null);

  const [formati, setFormati] = useState<FormatiDisponibili>({ video: [], testo: [] });
  const [formatoScelto, setFormatoScelto] = useState("h264");
  const [destinazione, setDestinazione] = useState("");
  const [qualita, setQualita] = useState(0);
  const [statoExport, setStatoExport] = useState<StatoExport>({ fase: "scelta" });

  const [stato, setStato] = useState<{ tono: Tono; testo: string }>({
    tono: "riposo",
    testo: "Nessun file caricato",
  });

  // La fase in corso, per la barra di stato e per la scheda.
  const faseInCorso = useRef<NomeFase | null>(null);

  // ------------------------------------------------------------ all'avvio
  useEffect(() => {
    void (async () => {
      setInfo(await ponte.informazioni());
      const i = await ponte.impostazioni();
      setImpostazioni(i);
      setImpostazioniSalvate(i);
      setModelli(await ponte.statoModelli());
      setCaratteri(await ponte.caratteri());
      const serie = await ponte.presetDiSerie();
      setPreset(serie[1] ?? serie[0] ?? null);
    })();
  }, []);

  // ------------------------------------------------------ gli eventi del motore
  useEffect(() => {
    let stop: (() => void) | null = null;
    void (async () => {
      stop = await ponte.ascolta((e: Evento) => applicaEvento(e));
    })();
    return () => {
      if (stop) stop();
    };
  }, []);

  const applicaEvento = useCallback((e: Evento) => {
    if (e.evento === "iniziata") {
      faseInCorso.current = e.fase;
      // L'ordine delle fasi e' quello della pipeline e non cambia: una fase
      // che comincia si accende dov'e', non salta in fondo all'elenco.
      setFasi((prima) =>
        prima.some((f) => f.fase === e.fase)
          ? prima.map((f) => (f.fase === e.fase ? { ...f, stato: "in-corso" } : f))
          : [...prima, { fase: e.fase, etichetta: ETICHETTE[e.fase], stato: "in-corso", secondi: 0 }],
      );
      setStato({ tono: "lavoro", testo: ETICHETTE[e.fase] });
    } else if (e.evento === "avanzamento") {
      setFrazione(e.frazione);
      setStatoExport((s) =>
        s.fase === "in-corso" && e.fase === "codifica" ? { ...s, frazione: e.frazione } : s,
      );
    } else if (e.evento === "conclusa") {
      setFasi((prima) =>
        prima.map((f) => (f.fase === e.fase ? { ...f, stato: "fatta", secondi: e.secondi } : f)),
      );
      setFrazione(0);
    } else if (e.evento === "avviso") {
      setStato({ tono: "riposo", testo: e.messaggio });
    } else if (e.evento === "annullata") {
      setStato({ tono: "riposo", testo: "Annullato" });
    }
  }, []);

  // -------------------------------------------------------- il trascinamento
  useEffect(() => {
    let stop: (() => void) | null = null;
    void (async () => {
      stop = await ponte.ascoltaTrascinamento(setSopra, (percorsi) => {
        if (percorsi.length > 0) void carica(percorsi[0]);
      });
    })();
    return () => {
      if (stop) stop();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ------------------------------------------------------------- caricamento
  const carica = useCallback(
    async (percorso: string) => {
      setSezione("carica");
      setParole([]);
      setRiepilogo(null);
      setTempo(0);
      setInRiproduzione(false);
      setElaborando(true);
      setFasi(
        FASI_TRASCRIZIONE.map((f) => ({
          fase: f,
          etichetta: ETICHETTE[f],
          stato: "attesa" as const,
          secondi: 0,
        })),
      );

      try {
        const d = await ponte.apri(percorso);
        setFile(d);
        setStato({ tono: "riposo", testo: `${d.nome} · ${d.riassunto}` });

        const r = await ponte.trascrivi();
        setRiepilogo(r);
        setParole(await ponte.parole());
        setOnda(await ponte.onda());
        setDimensioni(await ponte.dimensioni());
        const corrente = await ponte.presetCorrente();
        if (corrente) setPreset(corrente);
        setStato({
          tono: "ok",
          testo: `Trascrizione completata — ${r.parole.toLocaleString("it-IT")} parole in ${leggiDurata(r.secondi)} · ${r.dispositivo} · ${r.modello}`,
        });
        setFormati(await ponte.formati());
      } catch (e) {
        setStato({ tono: "errore", testo: String(e) });
      } finally {
        setElaborando(false);
        faseInCorso.current = null;
      }
    },
    [],
  );

  const scegliFile = useCallback(async () => {
    const scelto = await ponte.scegliFile(FILTRI_MEDIA);
    if (scelto) void carica(scelto);
  }, [carica]);

  // ------------------------------------------------------------- l'aspetto
  const applica = useCallback(async (nuovo: Preset) => {
    setPreset(nuovo);
    try {
      const avviso = await ponte.applicaAspetto(nuovo);
      setAvvisoCarattere(avviso);
      setDimensioni(await ponte.dimensioni());
    } catch (e) {
      setStato({ tono: "errore", testo: String(e) });
    }
  }, []);

  // ------------------------------------------------------------- l'export
  const scegliFormato = useCallback(async (id: string) => {
    setFormatoScelto(id);
    try {
      setDestinazione(await ponte.nomeProposto(id));
    } catch {
      /* senza file aperto non c'e' un nome da proporre */
    }
  }, []);

  useEffect(() => {
    if (sezione === "esporta" && !destinazione) void scegliFormato(formatoScelto);
  }, [sezione, destinazione, formatoScelto, scegliFormato]);

  const esporta = useCallback(async () => {
    const testuale = formati.testo.some((f) => f.id === formatoScelto);
    try {
      if (testuale) {
        const fatto = await ponte.esportaTesto(destinazione);
        setStatoExport({ fase: "fatto", percorso: fatto, fotogrammi: 0, secondi: 0 });
        setStato({ tono: "ok", testo: `Scritto ${fatto}` });
        return;
      }
      setStatoExport({ fase: "in-corso", frazione: 0, avvio: Date.now() });
      const esito = await ponte.esporta(formatoScelto, destinazione, qualita);
      setStatoExport({
        fase: "fatto",
        percorso: esito.percorso,
        fotogrammi: esito.fotogrammi,
        secondi: esito.secondi,
      });
      setStato({
        tono: "ok",
        testo: `Esportati ${esito.fotogrammi} fotogrammi in ${leggiDurata(esito.secondi)}`,
      });
    } catch (e) {
      setStatoExport({ fase: "scelta" });
      setStato({ tono: "errore", testo: String(e) });
    }
  }, [destinazione, formatoScelto, formati.testo, qualita]);

  // ------------------------------------------------------- le impostazioni
  const pendenti = useMemo(() => {
    if (!impostazioni || !impostazioniSalvate || !riepilogo) return false;
    return (
      impostazioni.modello !== impostazioniSalvate.modello ||
      impostazioni.lingua !== impostazioniSalvate.lingua ||
      impostazioni.dispositivo !== impostazioniSalvate.dispositivo ||
      impostazioni.termini !== impostazioniSalvate.termini
    );
  }, [impostazioni, impostazioniSalvate, riepilogo]);

  const cambiaImpostazioni = useCallback(async (d: DatiImpostazioni) => {
    setImpostazioni(d);
    await ponte.salvaImpostazioni(d);
    setModelli(await ponte.statoModelli());
    setCaratteri(await ponte.caratteri());
  }, []);

  const durataFile = file?.durata ?? 0;

  return (
    <div className="finestra">
      <BarraLaterale
        attiva={sezione}
        sbloccate={parole.length > 0}
        versione={info?.versione ?? "0.1.0"}
        repository={info?.repository ?? "https://github.com/zerflyne/verba"}
        onVai={setSezione}
      />

      {sezione === "carica" && (
        <Carica
          file={file}
          elaborando={elaborando}
          fasi={fasi}
          frazione={frazione}
          parole={parole}
          tempo={tempo}
          dimensioni={dimensioni}
          onda={onda}
          inRiproduzione={inRiproduzione}
          sopra={sopra}
          onScegli={() => void scegliFile()}
          onAnnulla={() => void ponte.annulla()}
          onTempo={setTempo}
          onRiproduzione={setInRiproduzione}
        />
      )}

      {sezione === "modifica" && preset && dimensioni && (
        <Modifica
          preset={preset}
          caratteri={caratteri}
          larghezza={dimensioni[0]}
          altezza={dimensioni[1]}
          tempo={tempo}
          durata={durataFile}
          inRiproduzione={inRiproduzione}
          avvisoCarattere={avvisoCarattere}
          onPreset={(p) => void applica(p)}
          onTempo={setTempo}
          onRiproduzione={setInRiproduzione}
          onSalvaPreset={() => {
            void (async () => {
              const dove = await ponte.scegliDoveSalvare("preset.json", [
                { name: "Preset di Verba", extensions: ["json"] },
              ]);
              if (dove && preset) await ponte.presetSalva(dove, preset);
            })();
          }}
          onCaricaPreset={() => {
            void (async () => {
              const da = await ponte.scegliFile([{ name: "Preset di Verba", extensions: ["json"] }]);
              if (da) void applica(await ponte.presetCarica(da));
            })();
          }}
          onPresetDiSerie={(nome) => {
            void (async () => {
              const serie = await ponte.presetDiSerie();
              const scelto = serie.find((p) => p.nome === nome);
              if (scelto) void applica(scelto);
            })();
          }}
          onAggiungiCarattere={() => {
            void (async () => {
              const f = await ponte.scegliFile([
                { name: "Carattere", extensions: ["ttf", "otf", "ttc"] },
              ]);
              if (!f) return;
              try {
                setCaratteri(await ponte.aggiungiCarattere(f));
                setStato({ tono: "ok", testo: "Carattere aggiunto" });
              } catch (e) {
                setStato({ tono: "errore", testo: String(e) });
              }
            })();
          }}
        />
      )}

      {sezione === "esporta" && (
        <Esporta
          formati={formati}
          scelto={formatoScelto}
          destinazione={destinazione}
          qualita={qualita}
          stato={statoExport}
          soloAudio={file?.modalita === "audio"}
          onScegliFormato={(id) => void scegliFormato(id)}
          onCambiaDestinazione={() => {
            void (async () => {
              const dove = await ponte.scegliDoveSalvare(destinazione, []);
              if (dove) setDestinazione(dove);
            })();
          }}
          onQualita={setQualita}
          onEsporta={() => void esporta()}
          onAnnulla={() => void ponte.annulla()}
          onApriCartella={() => {
            if (statoExport.fase === "fatto") void ponte.mostraNellaCartella(statoExport.percorso);
          }}
          onDiNuovo={() => setStatoExport({ fase: "scelta" })}
        />
      )}

      {sezione === "impostazioni" && impostazioni && (
        <Impostazioni
          dati={impostazioni}
          info={info}
          modelli={modelli}
          scaricando={scaricando}
          frazioneScarico={frazione}
          pendenti={pendenti}
          terminiLetti={null}
          onCambia={(d) => void cambiaImpostazioni(d)}
          onScegliTermini={() => {
            void (async () => {
              const f = await ponte.scegliFile([{ name: "CSV", extensions: ["csv", "txt"] }]);
              if (f) void cambiaImpostazioni({ ...impostazioni, termini: f });
            })();
          }}
          onScegliCartellaExport={() => {
            void (async () => {
              const d = await ponte.scegliCartella();
              if (d) void cambiaImpostazioni({ ...impostazioni, cartella_export: d });
            })();
          }}
          onScegliCartellaModelli={() => {
            void (async () => {
              const d = await ponte.scegliCartella();
              if (d) void cambiaImpostazioni({ ...impostazioni, cartella_modelli: d });
            })();
          }}
          onScarica={() => {
            void (async () => {
              setScaricando(true);
              try {
                setModelli(await ponte.scaricaModelli());
              } catch (e) {
                setStato({ tono: "errore", testo: String(e) });
              } finally {
                setScaricando(false);
              }
            })();
          }}
          onRitrascrivi={() => {
            setImpostazioniSalvate(impostazioni);
            if (file) void carica(file.percorso);
          }}
        />
      )}

      <BarraDiStato tono={stato.tono} testo={stato.testo} />
    </div>
  );
}
