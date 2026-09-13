# Le DLL di FFmpeg, solo per Windows

Qui dentro vanno le librerie di **FFmpeg** per Windows:

```
avcodec-61.dll  avformat-61.dll  avutil-59.dll
swresample-5.dll  swscale-8.dll  avfilter-10.dll  avdevice-61.dll  postproc-58.dll
```

Non e' la stessa storia di `lib/`. ONNX Runtime la carica `ort` a programma
gia' avviato, cercandola in piu' posti; FFmpeg invece e' **legato in modo
implicito** (`crates/verba-core/build.rs`), quindi a risolverla e' il
caricatore di Windows *prima* che una sola riga di Verba giri. Se manca non
c'e' errore da intercettare: si apre una finestra di sistema che dice
«Impossibile eseguire il codice perche' avcodec-61.dll non e' stato trovato»,
e il processo muore li'.

Per la stessa ragione le DLL devono stare **accanto all'eseguibile** e non in
una sottocartella: il caricatore guarda la cartella del `.exe`, non `lib/`. Ci
pensa `tauri.windows.conf.json`, che le copia nella radice delle risorse.

Su Linux e macOS questa cartella non serve e non viene letta: li' FFmpeg
arriva dal sistema (`libavcodec60` e compagne, dichiarate fra le dipendenze
del `.deb`).

Le mette qui il workflow di rilascio (`.github/workflows/rilascio.yml`), che
scarica la build *shared* di gyan.dev — la stessa da cui il `build.rs` prende
header e librerie d'importazione. Chi compila a mano su Windows deve metterle
qui da se': altrimenti `tauri build` si ferma su `ffmpeg/*.dll` e dice che il
modello non trova niente. E' voluto — meglio fermarsi qui che produrre un
installer che non si apre.

Questa cartella e' sotto controllo di versione solo per questo file: le `.dll`
non stanno in un repository git.
