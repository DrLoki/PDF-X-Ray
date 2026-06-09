# CLI Analyzer — `cli_analyze`

Strumento da riga di comando per eseguire l'analisi XY-Cut su una pagina PDF senza avviare l'applicazione Tauri. Utile per debug, regressioni automatizzate e ispezione dei risultati su file arbitrari.

---

## Compilazione

```powershell
cd src-tauri
cargo build --bin cli_analyze
# binario prodotto: src-tauri/target/debug/cli_analyze.exe
```

Build di release (più veloce in esecuzione):

```powershell
cargo build --release --bin cli_analyze
# binario prodotto: src-tauri/target/release/cli_analyze.exe
```

---

## Utilizzo

Il tool accetta JSON in ingresso tramite **stdin** oppure da un **file** passato come primo argomento, e scrive il risultato JSON su **stdout**.

```
cli_analyze [<input.json>]
```

| Modalità | Comando |
|---|---|
| Da file | `cli_analyze page6.json` |
| Da stdin | `cat page6.json \| cli_analyze` |
| Pipe completa | `node extract.js doc.pdf 6 \| cli_analyze` |

---

## Formato di input

```json
{
  "items": [
    {
      "text": "Chapter 10: Combat.",
      "x": 318.0,
      "y": 51.7,
      "width": 97.0,
      "height": 11.2,
      "font_size": 9.5,
      "font_name": "PlantinMTPro-Bold"
    }
  ],
  "pageBounds": { "x": 0, "y": 0, "w": 612.0, "h": 792.0 },
  "borderedBoxes": [],
  "strategy": "combined",
  "priority": "Y"
}
```

| Campo | Tipo | Obbligatorio | Descrizione |
|---|---|---|---|
| `items` | array | ✓ | Elementi testuali estratti dalla pagina |
| `items[].text` | string | ✓ | Contenuto testuale |
| `items[].x` | float | ✓ | Coordinata X del bordo sinistro |
| `items[].y` | float | ✓ | Coordinata Y del bordo superiore |
| `items[].width` | float | ✓ | Larghezza del bounding box |
| `items[].height` | float | ✓ | Altezza del bounding box |
| `items[].font_size` | float | ✓ | Dimensione del font in punti |
| `items[].font_name` | string\|null | — | Nome del font (opzionale) |
| `pageBounds` | object | ✓ | Dimensioni dell'intera pagina |
| `borderedBoxes` | array | ✓ | Rettangoli vettoriali (callout box). Array vuoto se non disponibili |
| `strategy` | string | — | Strategia di rilevamento (default: `"combined"`) |
| `priority` | string | — | Priorità di taglio (`"Y"`, `"X"`, o `"max-gap"`, default: `"Y"`) |

### Valori validi per `strategy`

| Valore | Descrizione |
|---|---|
| `"combined"` | Ibrido: max(delta-x histogram, dominant-font × 1.5). Default consigliato |
| `"delta-x"` | Analisi istogramma delle distanze orizzontali tra elementi adiacenti |
| `"zero-run"` | Soglie fisse (T_x=10, T_y=5): rileva qualsiasi corridoio bianco |
| `"dominant-font"` | Soglie proporzionali al font dominante (T_x = font × 1.5, T_y = font × 1.0) |

---

## Estrazione dati da PDF

Prima di passare il JSON al CLI, occorre estrarre gli elementi testuali dal PDF. Sono disponibili due tool:

### Python (consigliato — richiede PyMuPDF)

```powershell
pip install pymupdf

python tools/extract_page_py.py "C:/path/to/document.pdf" 6 > page6.json
```

### Node.js (richiede canvas nativo per DOMMatrix)

```powershell
npm ci
node tools/extract_page.js "C:/path/to/document.pdf" 6 > page6.json
```

> **Nota:** Con Node.js 22+ senza canvas nativo, `extract_page.js` fallisce per `DOMMatrix is not defined`. Usare il tool Python in questo caso.

---

## Pipeline completa (esempio)

Analisi della pagina 6 con tutte le strategie:

```powershell
# 1. Estrarre la pagina
python tools/extract_page_py.py "F:/path/document.pdf" 6 > page6.json

# 2. Analizzare con la strategia combined (default)
cat page6.json | src-tauri/target/debug/cli_analyze > result_combined.json

# 3. Analizzare con una strategia specifica
$input = Get-Content page6.json -Raw | ConvertFrom-Json
$input | Add-Member -NotePropertyName strategy -NotePropertyValue "zero-run" -Force
$input | ConvertTo-Json -Depth 20 -Compress | src-tauri/target/debug/cli_analyze > result_zero_run.json
```

Ispezione rapida della struttura rilevata:

```powershell
$result = Get-Content result_combined.json | ConvertFrom-Json
foreach ($c in $result.root.children) {
    Write-Host "[$($c.id)] type=$($c.type) cut=$($c.cutDirection) x=$([math]::Round($c.bounds.x)) w=$([math]::Round($c.bounds.w))"
}
```

---

## Formato di output

Il JSON in output segue la struttura `XYCutResult`:

```json
{
  "root": {
    "id": "block-root",
    "type": "root",
    "bounds": { "x": 0, "y": 0, "w": 612, "h": 792 },
    "depth": 0,
    "children": [
      {
        "id": "block-2",
        "type": "container",
        "cutDirection": "Y",
        "bounds": { "x": 57, "y": 0, "w": 239, "h": 792 },
        "depth": 1,
        "children": [ ... ]
      },
      {
        "id": "block-6",
        "type": "leaf",
        "bounds": { "x": 318, "y": 0, "w": 294, "h": 792 },
        "depth": 1,
        "text": "Chapter 10: Combat. ...",
        "formatting": {
          "fontFamily": "PlantinMTPro-Regular",
          "fontSize": 9.5,
          "avgFontSize": 9.5,
          "fontWeight": "normal",
          "fontStyle": "normal",
          "color": "#ffffff",
          "alignment": "left"
        }
      }
    ]
  },
  "projections": {
    "xGaps": [
      { "start": 296.2, "end": 318.0, "size": 21.8 }
    ],
    "yGaps": [
      { "start": 685.9, "end": 751.2, "size": 65.3 }
    ],
    "pageBounds": { "x": 0, "y": 0, "w": 612, "h": 792 }
  }
}
```

| Campo | Descrizione |
|---|---|
| `root` | Albero DOM ricorsivo della pagina |
| `root.type` | `"root"`, `"container"` o `"leaf"` |
| `root.cutDirection` | `"X"` (taglio verticale/colonne) o `"Y"` (taglio orizzontale/righe) |
| `root.children` | Figli del nodo (presenti su `root` e `container`) |
| `root.text` | Testo concatenato (solo su `leaf`) |
| `root.formatting` | Stile dominante (solo su `leaf`) |
| `projections.xGaps` | Gap orizzontali rilevati sull'intera pagina |
| `projections.yGaps` | Gap verticali rilevati sull'intera pagina |

---

## Codici di uscita

| Codice | Significato |
|---|---|
| `0` | Successo |
| `2` | Errore di parsing del JSON in input |
| `3` | Errore di serializzazione del risultato |
