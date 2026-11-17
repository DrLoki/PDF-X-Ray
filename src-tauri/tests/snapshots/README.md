# Snapshot di riferimento

Copia qui gli snapshot salvati dall'app (pulsante 📸 Snapshot, cartella Documenti/PDF-X-Ray/snapshots).
Il test `tests/snapshots.rs` li valuta; con `"minAutoScore": 0.9` uno snapshot diventa un test di regressione.

    cargo test --test snapshots -- --nocapture
    cargo run --features cli --bin cli_analyze -- --eval tests/snapshots
