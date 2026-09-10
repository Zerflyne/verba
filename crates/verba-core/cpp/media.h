// Lettura di un file multimediale: metadati e fotogrammi.
//
// Interfaccia C, come per l'encoder: il ponte con Rust passa da qui e non
// dalle strutture di libavformat.
#ifndef VERBA_MEDIA_H
#define VERBA_MEDIA_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct VerbaMedia VerbaMedia;

// Cio' che si sa di un file senza decodificarlo.
typedef struct {
    int ha_video;
    int ha_audio;
    int larghezza;
    int altezza;
    // Frame rate come frazione esatta: i valori NTSC non sono decimali.
    int fps_num;
    int fps_den;
    double durata;          // secondi; 0 se sconosciuta
    char codec_video[32];
    char codec_audio[32];
} VerbaInfo;

// Apre il file e riempie `info`. Ritorna NULL in caso di errore, scrivendo il
// motivo in `errore`.
VerbaMedia *verba_media_apri(const char *percorso, VerbaInfo *info,
                             char *errore, int errore_len);

// Decodifica il fotogramma visibile al tempo `t` e lo scrive in `rgba`
// (larghezza * altezza * 4 byte, opaco). `passo` e' la lunghezza di una riga
// in byte; 0 significa larghezza * 4.
//
// Ritorna 0 se ha scritto un fotogramma, 1 se oltre la fine del file, negativo
// in caso di errore.
int verba_media_fotogramma(VerbaMedia *m, double t, uint8_t *rgba, int passo,
                           char *errore, int errore_len);

void verba_media_libera(VerbaMedia *m);

#ifdef __cplusplus
}
#endif

#endif
