# Contribuire

- **Rami:** si parte da `develop` con un ramo breve e si apre una pull request verso `develop`. `main` è solo per le release, aperte dal proprietario.
- **Licenza dei contributi:** inviando una modifica accetti che sia distribuita con la licenza MIT del progetto. Non serve altro (nessun CLA).
- **Prima di aprire la pull request:**
  - server e libreria di calcolo: `cargo test --manifest-path server/Cargo.toml --locked --jobs 1` e lo stesso per `compute/`;
  - app: `cargo test --manifest-path app/Cargo.toml --locked --jobs 1 --no-default-features --features test-support`;
  - interfaccia: in `desktop/`, `npm ci && npm run check`.
- **Segreti:** non mettere mai chiavi, password o token veri nel codice, nei test o nei log. I valori dei test sono finti.
- **Sicurezza:** per una vulnerabilità non aprire una issue pubblica; scrivi al proprietario del repository in privato (GitHub, «Report a vulnerability»).
- **Marchio:** vedi `TRADEMARKS.md`.
