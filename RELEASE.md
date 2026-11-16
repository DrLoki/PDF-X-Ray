# Release Notes — v0.2.0

## Overview

Questa release cambia l'obiettivo dell'analisi: dalla sola segmentazione della pagina in blocchi alla ricostruzione del **flusso di lettura** e dei **paragrafi logici**, le unità corrette per la traduzione. Un paragrafo spezzato da cambio colonna, cambio pagina, box o figura viene ricomposto in un'unica unità. Header, footer, numeri di pagina, didascalie e note vengono riconosciuti e tenuti fuori dal flusso del testo principale. La traduzione può essere sovrapposta al PDF originale, ridistribuita sui blocchi di partenza.

Tutta l'analisi avviene ora nel motore Rust, sull'intero documento e in punti PDF, quindi i risultati non dipendono più dal livello di zoom.

---

## Novità e Miglioramenti

### 1. Nuova pipeline di analisi in Rust
Il vecchio modulo `gutter_detection.rs` è stato sostituito da una pipeline a moduli:

| Modulo | Compito |
|---|---|
| `lines.rs` | Ricostruzione delle righe dai frammenti di testo e gestione dei capilettera |
| `xycut.rs` | XY-Cut ricorsivo sulle righe, con box bordati e tabelle |
| `roles.rs` | Classificazione dei blocchi per ruolo |
| `order.rs` | Ordine di lettura |
| `flow.rs` | Suddivisione in paragrafi e cucitura tra blocchi e pagine |
| `reflow.rs` | Ridistribuzione del testo tradotto |
| `text.rs` | Normalizzazione del testo, sillabazione, euristiche lessicali |
| `pipeline.rs`, `model.rs` | Orchestrazione e modello dati |

### 2. Nessun testo perso
- Ogni riga viene assegnata a esattamente un blocco: i tagli si fanno in base al centro delle righe, e un gruppo troppo stretto viene unito al blocco vicino invece di essere scartato. In 0.1.1 il titolo centrato e il footer di `page5` sparivano.
- Le statistiche (`inputChars` / `outputChars`) permettono di verificare la conservazione del testo su ogni documento.

### 3. Ricostruzione del testo
- Le righe vengono costruite prima dell'XY-Cut, scorrendo i frammenti da sinistra a destra. Un gutter di colonna separa sempre le righe, e un elemento alto in una colonna non può più unire due righe dell'altra.
- Lo spazio tra frammenti si inserisce in base al distacco reale, non più sempre: `tra` + `duzione` → `traduzione`.
- Capilettera ricomposti: `T` + `he Cosmere` → `The Cosmere`.
- Le legature tipografiche (`ﬁ`, `ﬂ`, …) vengono espanse; i soft hyphen (U+00AD) vengono rimossi.
- Sillabazione a fine riga ricomposta: `tradu-` + `zione` → `traduzione`. Se il documento usa i soft hyphen per gli a capo, un `-` a fine riga è un composto reale e viene mantenuto (`storm-blasted`, `high-stakes`).

### 4. Segmentazione più robusta
- Fallback per colonne con elementi a cavallo del gutter: le righe che attraversano la linea di separazione vengono isolate con tagli orizzontali. Il fallback viene tentato anche quando i gap globali isolano solo elementi sottili, come un credito verticale a margine.
- Immagini e box fanno da ostacoli nelle proiezioni, quindi un taglio non può più attraversare una figura. Le immagini di sfondo con testo stampato sopra non sono ostacoli, così non nascondono il gutter tra le colonne.
- La soglia verticale automatica dipende dall'interlinea del blocco (3.5 × lo spazio mediano tra le righe, fra ½ e 1 em) invece di valere sempre 1 em: gli stacchi di sezione e lo spazio sopra i titoli vengono separati.
- I box bordati vengono riconosciuti di nuovo. Con pdf.js 6 il rilevamento dei rettangoli non funzionava più, perché il formato dei tracciati di `constructPath` è cambiato. Ora tiene conto della matrice di trasformazione (CTM) e dei form XObject.
- Le griglie di rettangoli adiacenti diventano un'unica tabella. Una campitura dietro una singola riga (ad esempio un titolo evidenziato) non viene trattata come box.

### 5. Ruoli dei blocchi
Ogni blocco riceve un ruolo: `body`, `heading`, `header`, `footer`, `page-number`, `caption`, `footnote`, `sidebar`, `table`, `figure-text`. I criteri sono:
- **header e footer**: posizione nei margini, confermata dalla ripetizione dello stesso testo alla stessa altezza su pagine vicine (le cifre e i numeri romani dei folii vengono ignorati nel confronto);
- **numeri di pagina**: schemi come `12`, `xii`, `Page 3`, `3 / 120`, `Pag. 7 di 30`;
- **didascalie**: prefissi come `Fig.`, `Figure`, `Tabella`, `Abb.`, `Fonte:`, oppure vicinanza a un'immagine con corpo minore o corsivo;
- **note a piè di pagina**: parte bassa della pagina, corpo ridotto, marcatore iniziale;
- **testo verticale** (crediti, etichette a margine): `figure-text`;
- **titoli**: corpo maggiore oppure riga interamente in grassetto. Un attacco in grassetto a inizio paragrafo ("**Chapter 1.** This walks you…") resta nel suo paragrafo.

### 6. Ordine di lettura
Ordinamento topologico secondo le regole di Breuel, con le immagini come separatori. L'ordine dell'albero XY-Cut, che dipende dalla priorità scelta, serve solo a decidere i casi di parità. Gli header vengono letti per primi; note, testo verticale, footer e numeri di pagina per ultimi.

### 7. Paragrafi logici e cucitura
- I paragrafi dentro un blocco si separano in base a interlinea, rientro, rientro sporgente, riga finale corta, elenchi puntati e cambi di corpo o di peso.
- I paragrafi interrotti da cambio colonna, cambio pagina o box vengono ricuciti. Didascalie, note e header che stanno in mezzo non interrompono la cucitura e vengono registrati come interruzioni.
- Ogni giunzione indica il tipo (`column`, `page`, `box`), se c'è stata un'interruzione e se è stata ricomposta una parola sillabata.

### 8. Supporto Tagged PDF
- Vengono letti i marked content (MCID), gli `/Artifact` e lo StructTree, ruoli compresi.
- Sulle pagine taggate l'ordine di lettura e le unità seguono la struttura: un elemento `P` diviso su due colonne resta un solo paragrafo. Gli `/Artifact` diventano header, footer o numeri di pagina.
- Il supporto può essere disattivato con l'opzione **Usa Tagged PDF**.

### 9. Overlay di traduzione
- Ogni paragrafo logico è un'unità di traduzione. Le unità si esportano in JSON (**Unità di traduzione · JSON**) e lo stesso file, con i campi `translation` compilati, si reimporta con **Importa**. Viene accettato anche il formato `{ "par12": "…" }`.
- Il testo tradotto viene ridistribuito sui pezzi del paragrafo originale, in proporzione alla loro lunghezza, preferendo i confini di frase.
- Ogni pezzo viene adattato al proprio box mantenendo interlinea, peso e allineamento. I box in cui il testo non entra comunque vengono evidenziati.
- Il pulsante **Pseudo** genera una pseudo-traduzione più lunga del 30%, per collaudare re-flow e adattamento senza un motore di traduzione.

### 10. Interfaccia
- Nuova tab **Paragrafi** con i paragrafi della pagina o dell'intero documento: ruolo, pagine coperte, numero di cuciture e stato della traduzione.
- Nuovo overlay **Flow**: numeri d'ordine di lettura e frecce di continuazione tra colonne (`↪ p.N` / `↩ p.N` tra pagine).
- I blocchi sono colorati per ruolo. L'inspector mostra ruolo, ordine, testo del blocco, testo ricomposto del paragrafo e traduzione.
- Il documento viene estratto in background con indicazione dell'avanzamento, a partire dalla pagina corrente.
- Rendering nitido sugli schermi HiDPI; la toolbar va a capo quando lo spazio non basta.
- **Barra di avanzamento** durante l'apertura: preparazione, estrazione pagina per pagina e analisi del layout, con il tempo stimato alla fine. La stima dell'analisi si affina con le analisi già eseguite nella sessione.
- Corretto l'errore all'apertura di un secondo PDF (`state.pdfDoc.destroy is not a function`): con pdf.js 6 il documento precedente si rilascia tramite il loading task.
- Nuovi formati di esportazione: documento completo in JSON, paragrafi in TXT, unità di traduzione in JSON. L'esportazione HTML ora applica l'escape al testo.

### 11. Snapshot per tarare le euristiche
- Pulsante **📸 Snapshot** nel pannello Layout Engine. Salva in `Documenti/PDF-X-Ray/snapshots` un JSON e un PNG della vista. Il JSON contiene:
  - i dati estratti della pagina e delle pagine vicine;
  - le impostazioni in uso;
  - il giudizio dell'utente (manuale meglio, automatico meglio, corretto, entrambi sbagliati) e una nota;
  - i risultati con le impostazioni correnti, con le soglie automatiche alla stessa priorità e con le impostazioni automatiche predefinite;
  - la segmentazione di riferimento validata.
- Diagnostica dei tagli: ogni nodo dell'albero (`diag`) riporta le soglie usate, il gap più grande su ciascun asse anche se sotto soglia, i gap candidati, i tentativi e la decisione finale.
- `cli_analyze --eval <cartella>` misura quanto le soglie automatiche riproducono i riferimenti (F1 a coppie di righe). Gli snapshot copiati in `src-tauri/tests/snapshots/` con `"minAutoScore"` diventano test di regressione.

### 12. CLI, strumenti e test
- `cli_analyze` accetta una singola pagina (formato precedente) o un documento `{ "pages": [...], "options": {...} }`.
- Nuove opzioni di `cli_analyze`:

| Opzione | Effetto |
|---|---|
| `--text` | stampa i paragrafi logici con ruolo e giunzioni |
| `--priority` | sceglie la priorità di taglio |
| `--strategy` | sceglie la strategia per le soglie |
| `--no-tags` | ignora le informazioni del Tagged PDF |
| `--no-stitch` | disattiva la cucitura dei paragrafi |
| `--distribute` | ridistribuisce un testo tradotto sui segmenti |
| `--eval` | valuta gli snapshot rispetto alla segmentazione di riferimento |

- `tools/extract_page.js` usa lo stesso estrattore dell'app (`src/extract.js`). Funziona con Node 22 e accetta una pagina, `all` o un intervallo (`10-14`).
- Nuova suite di test (`npm test`): test unitari per modulo e test di integrazione sui fixture (`src-tauri/tests/fixtures/`), con documenti sintetici multi-pagina e un caso Tagged PDF.
- Nuova documentazione `docs/layout-pipeline.md` (sostituisce `docs/gutter-detection.md`); aggiornati `docs/cli-analyze.md` e il README.

---

## Modifiche incompatibili

- **Comando Tauri**: `analyze_layout_auto_gutter` è sostituito da `analyze_document(pages, options)`, che restituisce un `DocumentResult` (pagine, paragrafi, statistiche). Il nuovo comando `distribute_translations` ridistribuisce i testi tradotti.
- **Unità di misura**: coordinate e soglie dei cursori sono in punti PDF, non più in pixel dello schermo. I valori di default di *Min Block Width* e *Min Block Height* sono ora 20 pt e 0 pt.
- **Formato di output**: i campi dei nodi JSON sono in camelCase. Le foglie hanno in più `role`, `readingOrder` e `paragraphIds`, e gli ID dei blocchi includono il numero di pagina (`p3-b12`).
- **Motore JavaScript rimosso**: `src/xycut.js` non esiste più e l'analisi richiede l'app desktop (Tauri).
- **Fixture spostati**: `page*.json` si trovano ora in `src-tauri/tests/fixtures/`.

---

## Limiti noti

- Le pagine fatte di elenchi "Etichetta: valore" senza punteggiatura (ad esempio i crediti) restano blocchi grandi.
- Nei documenti senza soft hyphen, un composto reale spezzato a fine riga perde il trattino: senza un dizionario non si distingue da una sillabazione.
- Le tabelle senza bordi non vengono riconosciute come tabelle.
- Le pagine scansionate non sono supportate (nessun OCR).

---

## Istruzioni di Build

Per compilare ed effettuare il packaging dell'applicazione in modalità di produzione:

```powershell
# Compilazione ed installazione delle dipendenze frontend
npm install

# Esecuzione dei test del motore di analisi
npm test

# Compilazione e creazione del pacchetto desktop definitivo (MSI / NSIS)
npm run tauri build
```

I pacchetti compilati e firmati verranno generati all'interno della cartella:
`src-tauri/target/release/bundle/`
