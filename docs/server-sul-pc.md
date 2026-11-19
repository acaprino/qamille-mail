# Il server di Qamille sul proprio PC

L'app non esegue modelli e non avvia server: ricerca per significato, estrazione dei documenti e trascrizione con Whisper passano da un server Qamille. Chi vuole tutto sul suo computer usa il server in Docker, con l'immagine pubblica già pronta. Senza server l'app resta un client di posta completo, con la ricerca per parole.

## 1. Docker Desktop su Windows

Installa Docker Desktop con il motore WSL2 (Windows 10 o 11, virtualizzazione attiva nel BIOS). Verifica con `docker version` e `docker compose version`.

## 2. Quanta memoria dare a Docker

Di base WSL2 prende al più metà della RAM del PC. Per cambiare il limite crea `%UserProfile%\.wslconfig`:

```
[wsl2]
memory=12GB
```

poi `wsl --shutdown` e riapri Docker Desktop. Indicativamente servono: EmbeddingGemma 2, circa 2 GB; Whisper, da circa 1 GB (modello piccolo) a circa 4 GB (modello grande); l'assistente interno (`llm_provider = builtin`), che si **somma** agli altri: `llm-piccolo` (Gemma 4 E4B) circa 6,9 GB (e 5,4 GB di disco), `llm-medio` (Gemma 4 12B) circa 11,6 GB (7,2 GB di disco), `llm-grande` (Gemma 4 26B-A4B) circa 19,3 GB (14,6 GB di disco), con il contesto predefinito di 8192 token e una richiesta alla volta. Somma ciò che usi e lascia memoria a Windows. Con `llm-piccolo` più EmbeddingGemma e Whisper piccolo, `memory=12GB` basta di norma: l'esempio qui sopra. Il server non avvia il modello se la memoria libera non basta, e risponde 503 invece di bloccarsi; nel compose c'è anche `mem_limit` per limitare solo il server.

## 3. La chiave amministratore

In `server/` copia `.env.local.example` in `.env` e scrivi dopo `QAMILLE_ADMIN_KEY=` una chiave lunga e non ripetitiva (il server rifiuta le chiavi brevi o ripetitive). Il file `.env` non si committa.

## 4. Avviare il server

```
docker compose -f compose.local.yaml up -d
```

## 5. Creare un utente

```
docker compose -f compose.local.yaml run --rm server user create <nome>
```

Il comando stampa la password provvisoria: l'app la chiede al primo accesso e ne fa scegliere una tua.

## 6. Scaricare i modelli

Apri `http://127.0.0.1:8788/admin`, accedi con la chiave amministratore e installa EmbeddingGemma e Whisper. Per l'assistente interno scrivi in `.env` `QAMILLE_LLM_PROVIDER=builtin`, `QAMILLE_LLM_BUILTIN_MODEL=llm-piccolo` e `QAMILLE_LLM_ACCEPT_LICENSE=gemma` (accetti le condizioni del modello) e riavvia il server: scarica da solo `llm-piccolo` in background (circa 5,4 GB; finché non ha finito, l'assistente risponde «in preparazione», e i progressi si vedono nella pagina `/admin`). Se preferisci installarlo a mano, lascia `QAMILLE_LLM_ACCEPT_LICENSE` vuota e installa `llm-piccolo` dalla stessa pagina (sezione «Modelli», che mostra memoria e disco richiesti accanto a quelli liberi, e la licenza del modello con l'indirizzo delle sue condizioni: Gemma 4 è Apache-2.0) oppure con `docker compose -f compose.local.yaml exec server qamille-server model install llm-piccolo`. Solo l'amministratore installa i modelli; l'app non può farlo. Poi nell'app, Calcolo, l'assistente è «questo server».

Se vuoi che le caselle di posta si ritrovino su ogni computer, genera una chiave una volta sola con `docker compose -f compose.local.yaml run --rm server sync keygen` e scrivila in `.env` come `QAMILLE_SYNC_KEY=...`: è la chiave con cui il server cifra le caselle salvate, e il backup di `server.db` da solo non basta a leggerle (conserva anche la chiave). Senza chiave la sincronizzazione è spenta.

## 7. Collegare l'app

Impostazioni, Calcolo, «Collegati a un server», indirizzo `http://127.0.0.1:8787`, username e password provvisoria (poi la tua). La pagina di consenso compare come per ogni server.

## 8. Aggiornare, fermare, cancellare

- Aggiornare: cambia il digest dell'immagine in `compose.local.yaml` con quello dell'ultima versione, poi `docker compose -f compose.local.yaml up -d`.
- Fermare: `docker compose -f compose.local.yaml down`.
- Cancellare anche i dati (utenti, modelli): `docker compose -f compose.local.yaml down -v`. Non si può annullare.

## 9. Podman

Podman con `podman compose` è un'alternativa compatibile: gli stessi file, gli stessi comandi con `podman` al posto di `docker`.

## 10. Perché 127.0.0.1 e non la rete

Il login in chiaro (senza TLS) vale solo dal computer stesso: le porte sono pubblicate su `127.0.0.1` e il server accetta il chiaro solo dal gateway della rete del compose. Per usarlo da altri computer serve un server con TLS (`server/compose.yaml`, con Caddy); non pubblicare mai le porte su `0.0.0.0`. `plaintext_from` è solo per una porta pubblicata su 127.0.0.1: non usarlo con una porta raggiungibile dalla rete.
