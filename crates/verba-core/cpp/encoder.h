/*
 * Encoder video, con e senza canale alfa.
 *
 * Interfaccia C (ABI stabile) sopra libavcodec + libavformat. Il chiamante
 * fornisce sempre frame RGBA8 con alfa "dritta" (non premoltiplicata), cosi'
 * come la producono i compositor e come la si attende un montaggio video; a
 * cosa diventino lo decide il formato.
 */
#ifndef VERBA_ENCODER_H
#define VERBA_ENCODER_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct SubEncoder SubEncoder;

/* I formati di uscita. */
typedef enum {
    /* MOV, prores_ks profilo 4444, yuva444p10le. L'unico profilo ProRes che
     * porta il canale alfa; 4:4:4 tiene netti i bordi del testo. */
    SUB_FORMATO_PRORES_4444 = 0,
    /* MOV, prores_ks profilo HQ, yuv422p10le. Senza alfa: per il video
     * sottotitolato di chi rimonta. */
    SUB_FORMATO_PRORES_422 = 1,
    /* MP4, libx264, yuv420p. Il formato che si puo' dare a chiunque. */
    SUB_FORMATO_H264 = 2,
    /* WebM, libvpx-vp9, yuva420p. Alfa in un decimo dello spazio del ProRes,
     * al prezzo di una codifica molto piu' lenta. */
    SUB_FORMATO_VP9_ALPHA = 3,
} SubFormato;

/* Vero se il formato trasporta il canale alfa. */
int sub_formato_ha_alfa(int formato);

/* L'estensione di file del formato, senza il punto. */
const char *sub_formato_estensione(int formato);

/*
 * Apre il file e prepara l'encoder.
 *
 * `qualita` significa cose diverse a seconda del formato: e' il quantizzatore
 * per i due ProRes (piu' basso = piu' bit; 4 e' il consigliato) e il CRF per
 * H.264 e VP9 (piu' basso = piu' bit; 18 e' il consigliato). Zero o meno
 * significa "il valore consigliato per questo formato".
 *
 * In caso di errore restituisce NULL e scrive il motivo in `errore`.
 */
/*
 * `audio_da`, se non NULL, e' il file da cui copiare la traccia audio: i
 * pacchetti vengono rimultiplexati senza ricodifica. Vale solo per i formati
 * senza alfa — un overlay trasparente non porta audio, altrimenti in montaggio
 * ci si ritroverebbe la stessa traccia due volte.
 */
SubEncoder *sub_encoder_apri(const char *percorso,
                             int formato,
                             const char *audio_da,
                             int larghezza,
                             int altezza,
                             int fps_num,
                             int fps_den,
                             int qualita,
                             int thread,
                             char *errore,
                             int errore_len);

/*
 * Converte un frame RGBA e lo scrive `ripetizioni` volte.
 *
 * La conversione avviene una sola volta: i sottotitoli restano identici per
 * molti frame consecutivi, e ripetere il frame gia' convertito evita di
 * rifare la trasformazione colore a ogni fotogramma.
 *
 * `passo` e' la lunghezza in byte di una riga di `rgba` (>= larghezza * 4).
 * Restituisce 0 se tutto e' andato bene, altrimenti un valore negativo.
 */
int sub_encoder_scrivi(SubEncoder *enc,
                       const uint8_t *rgba,
                       int passo,
                       int ripetizioni,
                       char *errore,
                       int errore_len);

/* Svuota l'encoder, scrive il trailer e chiude il file. */
int sub_encoder_chiudi(SubEncoder *enc, char *errore, int errore_len);

/* Libera la struttura. Va chiamata anche se `sub_encoder_chiudi` fallisce. */
void sub_encoder_libera(SubEncoder *enc);

/* Numero di frame scritti finora (utile per il log). */
int64_t sub_encoder_frame_scritti(const SubEncoder *enc);

#ifdef __cplusplus
}
#endif

#endif /* VERBA_ENCODER_H */
