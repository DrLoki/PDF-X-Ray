# Specifiche Tecniche: Algoritmo di Rilevamento Dinamico dei Gutter per XY-Cut

## 1. Obiettivo del Modulo

Il candidato AI deve sviluppare un modulo nativo in Rust integrato nel backend di un'applicazione Tauri. Il modulo ha lo scopo di analizzare geometricamente una pagina PDF strutturata (es. manuali a doppia colonna, layout complessi) e determinare automaticamente la presenza e l'ampiezza dei "gutter" (grondaie/spazi bianchi) orizzontali e verticali.
L'obiettivo finale è eliminare l'inserimento manuale delle dimensioni dei gutter da parte dell'utente, rendendo l'algoritmo XY-Cut Ricorsivo completamente autonomo ed adattivo.

## 2. Input e Struttura Dati di PartenzaIl codice riceverà in input un vettore piatto di elementi testuali estratti dalla pagina tramite parsing dei PDF nativi (es. tramite lopdf).

```Rust
pub struct TextElement {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub font_size: f32,
}
```

## 3. Strategie Core da Implementare

L'agente AI dovrà implementare un approccio ibrido combinato che unisce tre metodologie di analisi geometrica.

### Strategia 1: Analisi Statistica dei Delta-X (Istogramma delle Frequenze)

L'algoritmo non deve scansionare lo spazio vuoto in modo cieco, ma analizzare la distanza tra i blocchi di testo adiacenti.
- **Logica**: Per ogni riga visiva (stessa coordinata Y o range di tolleranza della linea di base), calcolare la distanza orizzontale tra la fine di un elemento (el.x + el.width) e l'inizio del successivo.
- **Calcolo della Soglia**: Raccogliere tutti i delta in un istogramma. Individuare i picchi di frequenza:
    - *Picco A (Kerning/Tracking)*: Spostamenti minimi inter-lettera.
    - *Picco B (Word Spacing)*: Spazio standard della barra spaziatrice.
    - *Picco C (Gutter)*: Spazi ampi isolati che separano le colonne.
- **Requisito**: Il codice deve identificare il minimo locale (la valle) che separa il Picco B dal Picco C per stabilire la soglia minima di un gutter valido.

### Strategia 2: Profilo di Proiezione Adattivo (Zero-Run Lengths)

Mentre l'istogramma analizza i singoli vicini, questa strategia analizza la continuità verticale dello spazio vuoto.
- **Logica**: Proiettare i bounding box dei `TextElement` sull'asse X (per i tagli verticali) e sull'asse Y (per i tagli orizzontali) creando un vettore di densità (array discreto con risoluzione minima di 1 punto tipografico).
- **Rilevamento**: Scansionare l'array alla ricerca di sequenze consecutive di zeri (aree senza testo), denominate Zero-Run Lengths.
- **Requisito**: Una sequenza di zeri è considerata un gutter di colonna valido solo se taglia l'intera altezza del macro-blocco analizzato in quel momento e se la sua larghezza supera la soglia dinamica calcolata dalla Strategia 3.

### Strategia 3: Euristica basata sul Font Dominante

La tipografia editoriale imposta i gutter proporzionalmente alla dimensione del testo del corpo principale.
- **Logica**: Calcolare la moda statistica del campo font_size all'interno del blocco analizzato per identificare il Font Dominante (es. testo dei paragrafi a 10pt).
- **Formula di Controllo**: Definire la tolleranza base del gutter come:
```math
\text{Gutter Minimo} = \text{Font Dominante} \times 1.5
```
- **Requisito**: Qualsiasi spazio bianco (Zero-Run Length) inferiore a questa soglia deve essere ignorato (trattato come tabulazione o rientro di paragrafo), evitando falsi tagli.

### 4. Workflow Logico del Parser (Algoritmo Richiesto)

Per ogni blocco sottoposto a XY-Cut (partendo dall'intera pagina come Root), l'agente AI deve eseguire i seguenti passaggi:
1. **Calcolo della Scala del Blocco**: Estrazione del font_size dominante e impostazione della soglia euristica minima.
2. **Generazione Istogramma Temporaneo**: Analisi dei delta tra elementi vicini per validare la presenza di strutture a colonna.
3. **Scansione Linee Vuote (Scan-line Adattiva)**:
    - Eseguire una proiezione globale sull'asse Y. Se viene trovata una sequenza di zeri valida (es. interlinea tra titolo e testo o spazio prima di una nota), eseguire un Y-Cut (taglio orizzontale).
    - Se non vengono trovati tagli orizzontali stabili, eseguire la proiezione sull'asse X per cercare una grondaia verticale.
4. **Validazione del Gutter Verticale**:
    - Se viene individuata una Zero-Run Length sull'asse X, verificare che la sua coordinata non coincida con i margini esterni della pagina.
    - Verificare se si trova nelle zone critiche di potenziale divisione (es. intorno a 1/2 per layout a due colonne, o 1/3 e 2/3 per layout a tre colonne).
5. **Esecuzione del Taglio e Ricorsione**: Dividere il set di TextElement nei rispettivi sotto-vettori in base alle coordinate del gutter individuato e riavviare il processo ricorsivamente su ogni sotto-blocco.

## 5. Gestione Casi Limite ed Eccezioni

L'agente AI dovrà implementare dei controlli specifici per evitare il fallimento dell'XY-Cut nei seguenti scenari:

| Caso Limite | Comportamento Atteso dell'Algoritmo |
| --- | --- |
| **Titoli Centrati Passanti** | L'algoritmo deve prioritariamente tentare il taglio orizzontale (Y-Cut). Isolatando il titolo in un blocco superiore autonomo, eviterà che la proiezione verticale successiva (X-Cut) tagli a metà il titolo stesso. |
| **Note a Piè di Pagina** | Identificare i blocchi nell'ultimo 15% inferiore della pagina con un font_size inferiore alla moda del documento. Isolarli preventivamente con un Y-Cut prima di analizzare i gutter delle colonne principali. |
| **Elementi grafici / Callout Box** | Se un blocco presenta coordinate sovrapposte che costringono il testo a una forma a "L" (aggiramento di un box), l'algoritmo deve rilevare la riduzione anomala dello spazio bianco e isolare il rettangolo del Box tramite i limiti dei bounding box adiacenti. |

## 6. Output Atteso

Il modulo Rust deve restituire un albero logico strutturato (DOM) esportabile in formato JSON, dove ogni nodo rappresenta un blocco coerente (Titolo, Colonna 1, Colonna 2, Nota) ordinato secondo il corretto flusso di lettura biologico.