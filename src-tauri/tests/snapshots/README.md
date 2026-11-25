# Snapshot di riferimento

Copia qui gli snapshot salvati dall'app (pulsante 📸 Snapshot, cartella Documenti/PDF-X-Ray/snapshots).
Il test `tests/snapshots.rs` li valuta; con `"minAutoScore": 0.9` uno snapshot diventa un test di regressione.

    cargo test --test snapshots -- --nocapture
    cargo run --example cli_analyze -- --eval tests/snapshots

Snapshot OCR: copia anche il file `<nome>_ocr.png` accanto al JSON. Con `"maxOcrCer": 0.01` nella sezione `ocr`
il test fallisce se il motore OCR attuale supera quel tasso di errore sui caratteri (solo con i modelli installati,
cartella dell'app o `PDFXRAY_MODEL_DIR`).

    cargo run --release --example cli_analyze -- --eval-ocr tests/snapshots

Snapshot di fumetti: i blocchi di riferimento sono i balloon nel loro ordine di lettura. Il test rifà il
rilevamento di vignette e balloon sull'immagine `_ocr.png` con le righe OCR salvate (non servono i modelli);
con `"minAutoScore"` fallisce se il raggruppamento scende sotto la soglia.

    cargo run --release --example cli_analyze -- --eval-comic tests/snapshots
